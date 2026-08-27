mod types;

pub use types::{
    CANONICAL_PREFLIGHT_SCHEMA_V1, CanonicalPreflightKeyV1, CanonicalProposalManifestV1,
    CanonicalProposalPreflightV1, CanonicalStatelessInputV1, chain_rules_fingerprint,
    proposal_event_digest,
};

use alloy_primitives::B256;
use anyhow::Result;
use async_trait::async_trait;
use raiko2_primitives::{L2BlockRange, RaikoError, RaikoResult, ShastaCheckpoint};
use raiko2_protocol::BlobProofType;
use raiko2_protocol_shasta::shasta::ShastaEventData;
use std::{
    collections::HashMap,
    future::Future,
    hash::{Hash, Hasher},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use tracing::warn;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalPreflightObject {
    pub key_digest: B256,
    pub content_hash: String,
    pub generation: Option<i64>,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalPreflightDescriptor {
    pub key_digest: B256,
    /// Identifies the payload for conflict reporting and observability. Exact
    /// deletion is fenced by `key_digest` plus the storage generation, so a
    /// remote store does not need to download the payload before a CAS delete.
    pub content_hash: String,
    pub generation: Option<i64>,
}

impl CanonicalPreflightDescriptor {
    #[must_use]
    pub fn same_storage_version(&self, other: &Self) -> bool {
        self.key_digest == other.key_digest
            && self.generation.is_some()
            && self.generation == other.generation
    }
}

impl CanonicalPreflightObject {
    #[must_use]
    pub fn descriptor(&self) -> CanonicalPreflightDescriptor {
        CanonicalPreflightDescriptor {
            key_digest: self.key_digest,
            content_hash: self.content_hash.clone(),
            generation: self.generation,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonicalPreflightPutResult {
    Created(CanonicalPreflightObject),
    AlreadyExists(CanonicalPreflightObject),
    Conflict(CanonicalPreflightDescriptor),
}

impl CanonicalPreflightPutResult {
    #[must_use]
    pub const fn try_object(&self) -> Option<&CanonicalPreflightObject> {
        match self {
            Self::Created(object) | Self::AlreadyExists(object) => Some(object),
            Self::Conflict(_) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanonicalPreflightDeleteResult {
    Removed,
    Missing,
    Stale,
}

#[async_trait]
pub trait CanonicalPreflightStore: std::fmt::Debug + Send + Sync {
    async fn get_canonical_preflight(
        &self,
        key: &CanonicalPreflightKeyV1,
    ) -> Result<Option<CanonicalPreflightObject>>;

    async fn put_canonical_preflight_if_absent(
        &self,
        key: &CanonicalPreflightKeyV1,
        bytes: &[u8],
    ) -> Result<CanonicalPreflightPutResult>;

    async fn delete_canonical_preflight_exact(
        &self,
        key: &CanonicalPreflightKeyV1,
        descriptor: &CanonicalPreflightDescriptor,
    ) -> Result<CanonicalPreflightDeleteResult>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreflightCacheResult {
    Hit,
    Miss,
    Bypass,
    Error,
}

impl PreflightCacheResult {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
            Self::Bypass => "bypass",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreflightCacheStage {
    Load,
    Build,
    Validate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreflightCacheRecoveryEvent {
    InvalidEntry,
    Rebuild,
    ExactDeleteRemoved,
    ExactDeleteMissing,
    ExactDeleteStale,
    ExactDeleteFailure,
    UncachedFallback,
}

impl PreflightCacheRecoveryEvent {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidEntry => "invalid_entry",
            Self::Rebuild => "rebuild",
            Self::ExactDeleteRemoved => "exact_delete_removed",
            Self::ExactDeleteMissing => "exact_delete_missing",
            Self::ExactDeleteStale => "exact_delete_stale",
            Self::ExactDeleteFailure => "exact_delete_failure",
            Self::UncachedFallback => "uncached_fallback",
        }
    }
}

impl PreflightCacheStage {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Load => "load",
            Self::Build => "build",
            Self::Validate => "validate",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreflightSingleFlightPhase {
    Locator,
    Core,
}

impl PreflightSingleFlightPhase {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Locator => "locator",
            Self::Core => "core",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreflightSingleFlightEvent {
    LeaderStarted,
    WaiterStarted,
    WaiterFinished,
}

enum CanonicalLoad {
    Hit(Box<CanonicalProposalPreflightV1>),
    Miss { publish: bool },
}

pub trait PreflightObserver: std::fmt::Debug + Send + Sync {
    fn record_cache_result(&self, _result: PreflightCacheResult) {}
    fn record_stage_duration(&self, _stage: PreflightCacheStage, _duration: Duration) {}
    fn record_serialized_size(&self, _bytes: usize) {}
    fn record_single_flight(
        &self,
        _phase: PreflightSingleFlightPhase,
        _event: PreflightSingleFlightEvent,
    ) {
    }
    fn record_recovery(&self, _event: PreflightCacheRecoveryEvent) {}
}

#[derive(Debug)]
struct NoopPreflightObserver;

impl PreflightObserver for NoopPreflightObserver {}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CanonicalPreflightLocatorKeyV1 {
    pub schema: u16,
    pub blob_proof_type: BlobProofType,
    pub l1_chain_id: u64,
    pub l2_chain_id: u64,
    pub proposal_id: u64,
    pub l2_block_range: L2BlockRange,
    pub l1_inclusion_block_number: u64,
    pub last_anchor_block_number: u64,
    pub checkpoint: Option<ShastaCheckpoint>,
    pub chain_rules_fingerprint: B256,
}

#[derive(Clone, Debug)]
pub struct CanonicalPreflightLocatorV1 {
    pub key: CanonicalPreflightKeyV1,
    pub block_numbers: Vec<u64>,
    pub expected_proposal_id: u64,
    pub proposal_event: ShastaEventData,
}

#[derive(Debug)]
pub struct PreflightCoordinator {
    store: Option<Arc<dyn CanonicalPreflightStore>>,
    observer: Arc<dyn PreflightObserver>,
    locator_flights: SingleFlight<CanonicalPreflightLocatorKeyV1, CanonicalPreflightLocatorV1>,
    canonical_flights: SingleFlight<CanonicalFlightKey, CanonicalProposalPreflightV1>,
}

impl PreflightCoordinator {
    #[must_use]
    pub fn new(store: Arc<dyn CanonicalPreflightStore>) -> Self {
        Self::with_observer(store, Arc::new(NoopPreflightObserver))
    }

    #[must_use]
    pub fn with_observer(
        store: Arc<dyn CanonicalPreflightStore>,
        observer: Arc<dyn PreflightObserver>,
    ) -> Self {
        Self {
            store: Some(store),
            observer,
            locator_flights: SingleFlight::default(),
            canonical_flights: SingleFlight::default(),
        }
    }

    #[must_use]
    pub fn disabled() -> Self {
        Self {
            store: None,
            observer: Arc::new(NoopPreflightObserver),
            locator_flights: SingleFlight::default(),
            canonical_flights: SingleFlight::default(),
        }
    }

    pub fn record_bypass(&self) {
        self.observer
            .record_cache_result(PreflightCacheResult::Bypass);
    }

    /// Resolves one lane-independent proposal locator through local single-flight.
    ///
    /// # Errors
    ///
    /// Returns the leader's locator construction error.
    pub async fn locate<Build, BuildFuture>(
        &self,
        key: CanonicalPreflightLocatorKeyV1,
        build: Build,
    ) -> RaikoResult<Arc<CanonicalPreflightLocatorV1>>
    where
        Build: Fn() -> BuildFuture + Send + Sync,
        BuildFuture: Future<Output = RaikoResult<CanonicalPreflightLocatorV1>> + Send,
    {
        self.locator_flights
            .run_observed(
                key,
                build,
                |event| {
                    self.observer
                        .record_single_flight(PreflightSingleFlightPhase::Locator, event);
                },
                shared_preflight_failure,
            )
            .await
    }

    /// Loads or builds one validated canonical preflight core through local single-flight.
    ///
    /// # Errors
    ///
    /// Returns key serialization, canonical build, or canonical validation errors.
    pub async fn canonical<Build, BuildFuture, Validate>(
        &self,
        key: CanonicalPreflightKeyV1,
        build: Build,
        validate: Validate,
    ) -> RaikoResult<Arc<CanonicalProposalPreflightV1>>
    where
        Build: Fn() -> BuildFuture + Send + Sync,
        BuildFuture: Future<Output = RaikoResult<CanonicalProposalPreflightV1>> + Send,
        Validate: Fn(&CanonicalProposalPreflightV1) -> RaikoResult<()> + Send + Sync,
    {
        let flight_key = CanonicalFlightKey::new(key.clone())?;
        self.canonical_flights
            .run_observed(
                flight_key,
                || self.load_or_build_canonical(&key, &build, &validate),
                |event| {
                    self.observer
                        .record_single_flight(PreflightSingleFlightPhase::Core, event);
                },
                shared_preflight_failure,
            )
            .await
    }

    async fn load_or_build_canonical<Build, BuildFuture, Validate>(
        &self,
        key: &CanonicalPreflightKeyV1,
        build: &Build,
        validate: &Validate,
    ) -> RaikoResult<CanonicalProposalPreflightV1>
    where
        Build: Fn() -> BuildFuture + Send + Sync,
        BuildFuture: Future<Output = RaikoResult<CanonicalProposalPreflightV1>> + Send,
        Validate: Fn(&CanonicalProposalPreflightV1) -> RaikoResult<()> + Send + Sync,
    {
        let publish = match self.try_load_canonical(key, validate).await {
            CanonicalLoad::Hit(core) => return Ok(*core),
            CanonicalLoad::Miss { publish } => publish,
        };

        self.observer
            .record_recovery(PreflightCacheRecoveryEvent::Rebuild);
        let build_started_at = Instant::now();
        let build_result = build().await;
        self.observer
            .record_stage_duration(PreflightCacheStage::Build, build_started_at.elapsed());
        let core = build_result?;
        let validation_started_at = Instant::now();
        let validation_result = validate(&core);
        self.observer.record_stage_duration(
            PreflightCacheStage::Validate,
            validation_started_at.elapsed(),
        );
        validation_result?;
        let Some(store) = self.store.as_ref().filter(|_| publish) else {
            if self.store.is_some() && !publish {
                self.observer
                    .record_recovery(PreflightCacheRecoveryEvent::UncachedFallback);
            }
            return Ok(core);
        };
        let bytes = match bincode::serialize(&core) {
            Ok(bytes) => bytes,
            Err(error) => {
                warn!(
                    proposal_id = key.proposal_id,
                    error = %error,
                    "canonical preflight serialization failed; continuing without cache publication"
                );
                self.observer
                    .record_recovery(PreflightCacheRecoveryEvent::UncachedFallback);
                return Ok(core);
            }
        };
        self.observer.record_serialized_size(bytes.len());
        Ok(self
            .publish_canonical(store, key, core, &bytes, validate)
            .await)
    }

    async fn try_load_canonical<Validate>(
        &self,
        key: &CanonicalPreflightKeyV1,
        validate: &Validate,
    ) -> CanonicalLoad
    where
        Validate: Fn(&CanonicalProposalPreflightV1) -> RaikoResult<()>,
    {
        let Some(store) = self.store.as_ref() else {
            self.observer
                .record_cache_result(PreflightCacheResult::Bypass);
            return CanonicalLoad::Miss { publish: false };
        };
        let load_started_at = Instant::now();
        let loaded = store.get_canonical_preflight(key).await;
        self.observer
            .record_stage_duration(PreflightCacheStage::Load, load_started_at.elapsed());
        match loaded {
            Ok(Some(object)) => match self.decode_and_validate(key, &object, validate) {
                Ok(core) => {
                    self.observer.record_cache_result(PreflightCacheResult::Hit);
                    self.observer.record_serialized_size(object.bytes.len());
                    CanonicalLoad::Hit(Box::new(core))
                }
                Err(error) => {
                    self.observer
                        .record_cache_result(PreflightCacheResult::Error);
                    self.observer
                        .record_recovery(PreflightCacheRecoveryEvent::InvalidEntry);
                    warn!(
                        proposal_id = key.proposal_id,
                        error = %error,
                        "invalidating unusable canonical preflight cache entry"
                    );
                    self.delete_unusable_and_resolve(store, key, &object, validate)
                        .await
                }
            },
            Ok(None) => {
                self.observer
                    .record_cache_result(PreflightCacheResult::Miss);
                CanonicalLoad::Miss { publish: true }
            }
            Err(error) => {
                self.observer
                    .record_cache_result(PreflightCacheResult::Error);
                warn!(
                    proposal_id = key.proposal_id,
                    error = %error,
                    "canonical preflight cache read failed; rebuilding"
                );
                CanonicalLoad::Miss { publish: false }
            }
        }
    }

    async fn publish_canonical<Validate>(
        &self,
        store: &Arc<dyn CanonicalPreflightStore>,
        key: &CanonicalPreflightKeyV1,
        core: CanonicalProposalPreflightV1,
        bytes: &[u8],
        validate: &Validate,
    ) -> CanonicalProposalPreflightV1
    where
        Validate: Fn(&CanonicalProposalPreflightV1) -> RaikoResult<()>,
    {
        match store.put_canonical_preflight_if_absent(key, bytes).await {
            Ok(CanonicalPreflightPutResult::Created(_)) => core,
            Ok(CanonicalPreflightPutResult::AlreadyExists(object)) => {
                match self.decode_and_validate(key, &object, validate) {
                    Ok(winner) => winner,
                    Err(error) => {
                        warn!(
                            proposal_id = key.proposal_id,
                            error = %error,
                            "discarding unusable canonical preflight publication winner"
                        );
                        self.delete_unusable(store, key, &object).await;
                        core
                    }
                }
            }
            Ok(CanonicalPreflightPutResult::Conflict(descriptor)) => {
                self.resolve_conflicting_winner(store, key, core, &descriptor, validate)
                    .await
            }
            Err(error) => {
                warn!(
                    proposal_id = key.proposal_id,
                    error = %error,
                    "canonical preflight cache publication failed; continuing"
                );
                self.observer
                    .record_recovery(PreflightCacheRecoveryEvent::UncachedFallback);
                core
            }
        }
    }

    async fn resolve_conflicting_winner<Validate>(
        &self,
        store: &Arc<dyn CanonicalPreflightStore>,
        key: &CanonicalPreflightKeyV1,
        core: CanonicalProposalPreflightV1,
        descriptor: &CanonicalPreflightDescriptor,
        validate: &Validate,
    ) -> CanonicalProposalPreflightV1
    where
        Validate: Fn(&CanonicalProposalPreflightV1) -> RaikoResult<()>,
    {
        match store.get_canonical_preflight(key).await {
            Ok(Some(winner)) => match self.decode_and_validate(key, &winner, validate) {
                Ok(winner) => winner,
                Err(error) => {
                    warn!(
                        proposal_id = key.proposal_id,
                        error = %error,
                        "discarding invalid conflicting canonical preflight winner"
                    );
                    self.delete_unusable(store, key, &winner).await;
                    core
                }
            },
            Ok(None) => {
                warn!(
                    proposal_id = key.proposal_id,
                    content_hash = %descriptor.content_hash,
                    "canonical preflight publication conflicted but winner disappeared"
                );
                self.observer
                    .record_recovery(PreflightCacheRecoveryEvent::UncachedFallback);
                core
            }
            Err(error) => {
                warn!(
                    proposal_id = key.proposal_id,
                    error = %error,
                    "failed to load canonical preflight publication winner"
                );
                self.observer
                    .record_recovery(PreflightCacheRecoveryEvent::UncachedFallback);
                core
            }
        }
    }

    fn decode_and_validate<Validate>(
        &self,
        key: &CanonicalPreflightKeyV1,
        object: &CanonicalPreflightObject,
        validate: &Validate,
    ) -> RaikoResult<CanonicalProposalPreflightV1>
    where
        Validate: Fn(&CanonicalProposalPreflightV1) -> RaikoResult<()>,
    {
        let expected_digest = key
            .digest()
            .map_err(|error| RaikoError::Serialization(error.to_string()))?;
        if object.key_digest != expected_digest {
            return Err(RaikoError::Preflight(
                "canonical preflight object key digest mismatch".to_string(),
            ));
        }
        let core: CanonicalProposalPreflightV1 = bincode::deserialize(&object.bytes)
            .map_err(|error| RaikoError::Serialization(error.to_string()))?;
        let validation_started_at = Instant::now();
        let result = validate(&core);
        self.observer.record_stage_duration(
            PreflightCacheStage::Validate,
            validation_started_at.elapsed(),
        );
        result?;
        Ok(core)
    }

    async fn delete_unusable_and_resolve<Validate>(
        &self,
        store: &Arc<dyn CanonicalPreflightStore>,
        key: &CanonicalPreflightKeyV1,
        object: &CanonicalPreflightObject,
        validate: &Validate,
    ) -> CanonicalLoad
    where
        Validate: Fn(&CanonicalProposalPreflightV1) -> RaikoResult<()>,
    {
        match store
            .delete_canonical_preflight_exact(key, &object.descriptor())
            .await
        {
            Ok(CanonicalPreflightDeleteResult::Removed) => {
                self.observer
                    .record_recovery(PreflightCacheRecoveryEvent::ExactDeleteRemoved);
                CanonicalLoad::Miss { publish: true }
            }
            Ok(CanonicalPreflightDeleteResult::Missing) => {
                self.observer
                    .record_recovery(PreflightCacheRecoveryEvent::ExactDeleteMissing);
                CanonicalLoad::Miss { publish: true }
            }
            Ok(CanonicalPreflightDeleteResult::Stale) => {
                self.observer
                    .record_recovery(PreflightCacheRecoveryEvent::ExactDeleteStale);
                match store.get_canonical_preflight(key).await {
                    Ok(Some(winner)) => match self.decode_and_validate(key, &winner, validate) {
                        Ok(core) => CanonicalLoad::Hit(Box::new(core)),
                        Err(error) => {
                            warn!(
                                proposal_id = key.proposal_id,
                                error = %error,
                                "replacement canonical preflight cache entry is unusable; rebuilding without publication"
                            );
                            CanonicalLoad::Miss { publish: false }
                        }
                    },
                    Ok(None) => CanonicalLoad::Miss { publish: true },
                    Err(error) => {
                        warn!(
                            proposal_id = key.proposal_id,
                            error = %error,
                            "failed to reload canonical preflight after stale exact deletion"
                        );
                        CanonicalLoad::Miss { publish: false }
                    }
                }
            }
            Err(error) => {
                self.observer
                    .record_recovery(PreflightCacheRecoveryEvent::ExactDeleteFailure);
                warn!(
                    proposal_id = key.proposal_id,
                    error = %error,
                    "failed to delete unusable canonical preflight cache entry; rebuilding without publication"
                );
                CanonicalLoad::Miss { publish: false }
            }
        }
    }

    async fn delete_unusable(
        &self,
        store: &Arc<dyn CanonicalPreflightStore>,
        key: &CanonicalPreflightKeyV1,
        object: &CanonicalPreflightObject,
    ) {
        self.observer
            .record_recovery(PreflightCacheRecoveryEvent::InvalidEntry);
        self.observer
            .record_recovery(PreflightCacheRecoveryEvent::UncachedFallback);
        match store
            .delete_canonical_preflight_exact(key, &object.descriptor())
            .await
        {
            Ok(CanonicalPreflightDeleteResult::Removed) => self
                .observer
                .record_recovery(PreflightCacheRecoveryEvent::ExactDeleteRemoved),
            Ok(CanonicalPreflightDeleteResult::Missing) => self
                .observer
                .record_recovery(PreflightCacheRecoveryEvent::ExactDeleteMissing),
            Ok(CanonicalPreflightDeleteResult::Stale) => self
                .observer
                .record_recovery(PreflightCacheRecoveryEvent::ExactDeleteStale),
            Err(error) => {
                self.observer
                    .record_recovery(PreflightCacheRecoveryEvent::ExactDeleteFailure);
                warn!(
                    proposal_id = key.proposal_id,
                    error = %error,
                    "failed to delete unusable canonical preflight cache entry"
                );
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CanonicalFlightKey {
    digest: B256,
    key: CanonicalPreflightKeyV1,
}

impl CanonicalFlightKey {
    fn new(key: CanonicalPreflightKeyV1) -> RaikoResult<Self> {
        let digest = key
            .digest()
            .map_err(|error| RaikoError::Serialization(error.to_string()))?;
        Ok(Self { digest, key })
    }
}

impl Hash for CanonicalFlightKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.digest.hash(state);
    }
}

struct SingleFlight<K, V> {
    flights: Arc<Mutex<HashMap<K, Arc<Flight<V>>>>>,
}

impl<K, V> std::fmt::Debug for SingleFlight<K, V> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SingleFlight")
            .field("in_flight", &lock_unpoisoned(&self.flights).len())
            .finish()
    }
}

impl<K, V> Default for SingleFlight<K, V> {
    fn default() -> Self {
        Self {
            flights: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl<K, V> SingleFlight<K, V>
where
    K: Clone + Eq + Hash,
{
    #[cfg(test)]
    async fn run<F, Fut>(&self, key: K, operation: F) -> std::result::Result<Arc<V>, anyhow::Error>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = std::result::Result<V, anyhow::Error>> + Send,
    {
        self.run_observed(
            key,
            operation,
            |_| {},
            |message| anyhow::anyhow!(message.to_string()),
        )
        .await
    }

    async fn run_observed<F, Fut, E, Observe, SharedError>(
        &self,
        key: K,
        operation: F,
        observe: Observe,
        shared_error: SharedError,
    ) -> std::result::Result<Arc<V>, E>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: Future<Output = std::result::Result<V, E>> + Send,
        E: std::fmt::Display,
        Observe: Fn(PreflightSingleFlightEvent) + Send + Sync,
        SharedError: Fn(&str) -> E + Send + Sync,
    {
        loop {
            match self.elect(key.clone()) {
                SingleFlightRole::Leader(leader) => {
                    observe(PreflightSingleFlightEvent::LeaderStarted);
                    return match operation().await {
                        Ok(value) => {
                            let value = Arc::new(value);
                            leader.finish(FlightState::Succeeded(Arc::clone(&value)));
                            Ok(value)
                        }
                        Err(error) => {
                            leader.finish(FlightState::Failed(Arc::from(error.to_string())));
                            Err(error)
                        }
                    };
                }
                SingleFlightRole::Follower(flight) => {
                    observe(PreflightSingleFlightEvent::WaiterStarted);
                    let waiter = SingleFlightWaiterObservation::new(&observe);
                    let result = flight.wait().await;
                    drop(waiter);
                    match result {
                        FlightWait::Succeeded(value) => return Ok(value),
                        FlightWait::Failed(message) => return Err(shared_error(&message)),
                        FlightWait::Retry => {}
                    }
                }
            }
        }
    }

    fn elect(&self, key: K) -> SingleFlightRole<K, V> {
        let mut flights = lock_unpoisoned(&self.flights);
        if let Some(flight) = flights.get(&key) {
            return SingleFlightRole::Follower(Arc::clone(flight));
        }
        let flight = Arc::new(Flight::new());
        flights.insert(key.clone(), Arc::clone(&flight));
        SingleFlightRole::Leader(SingleFlightLeader {
            key,
            flight,
            flights: Arc::clone(&self.flights),
            active: true,
        })
    }

    #[cfg(test)]
    fn in_flight_len(&self) -> usize {
        lock_unpoisoned(&self.flights).len()
    }
}

enum SingleFlightRole<K, V>
where
    K: Eq + Hash,
{
    Leader(SingleFlightLeader<K, V>),
    Follower(Arc<Flight<V>>),
}

struct SingleFlightLeader<K, V>
where
    K: Eq + Hash,
{
    key: K,
    flight: Arc<Flight<V>>,
    flights: Arc<Mutex<HashMap<K, Arc<Flight<V>>>>>,
    active: bool,
}

impl<K, V> SingleFlightLeader<K, V>
where
    K: Eq + Hash,
{
    fn finish(mut self, state: FlightState<V>) {
        self.flight.set_state(state);
        self.remove_current();
        self.flight.notify.notify_waiters();
        self.active = false;
    }

    fn remove_current(&self) {
        let mut flights = lock_unpoisoned(&self.flights);
        if flights
            .get(&self.key)
            .is_some_and(|current| Arc::ptr_eq(current, &self.flight))
        {
            flights.remove(&self.key);
        }
    }
}

impl<K, V> Drop for SingleFlightLeader<K, V>
where
    K: Eq + Hash,
{
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        self.flight.set_state(FlightState::Cancelled);
        self.remove_current();
        self.flight.notify.notify_waiters();
    }
}

struct Flight<V> {
    state: Mutex<FlightState<V>>,
    notify: Notify,
}

impl<V> Flight<V> {
    fn new() -> Self {
        Self {
            state: Mutex::new(FlightState::Running),
            notify: Notify::new(),
        }
    }

    fn set_state(&self, state: FlightState<V>) {
        *lock_unpoisoned(&self.state) = state;
    }

    async fn wait(&self) -> FlightWait<V> {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let snapshot = {
                let state = lock_unpoisoned(&self.state);
                match &*state {
                    FlightState::Running => FlightSnapshot::Wait,
                    FlightState::Succeeded(value) => FlightSnapshot::Succeeded(Arc::clone(value)),
                    FlightState::Failed(message) => FlightSnapshot::Failed(Arc::clone(message)),
                    FlightState::Cancelled => FlightSnapshot::Retry,
                }
            };
            match snapshot {
                FlightSnapshot::Wait => notified.await,
                FlightSnapshot::Succeeded(value) => {
                    return FlightWait::Succeeded(value);
                }
                FlightSnapshot::Failed(message) => return FlightWait::Failed(message),
                FlightSnapshot::Retry => return FlightWait::Retry,
            }
        }
    }
}

enum FlightState<V> {
    Running,
    Succeeded(Arc<V>),
    Failed(Arc<str>),
    Cancelled,
}

enum FlightWait<V> {
    Succeeded(Arc<V>),
    Failed(Arc<str>),
    Retry,
}

enum FlightSnapshot<V> {
    Wait,
    Succeeded(Arc<V>),
    Failed(Arc<str>),
    Retry,
}

fn shared_preflight_failure(message: &str) -> RaikoError {
    RaikoError::Preflight(format!("shared preflight leader failed: {message}"))
}

struct SingleFlightWaiterObservation<'a, Observe>
where
    Observe: Fn(PreflightSingleFlightEvent),
{
    observe: &'a Observe,
}

impl<'a, Observe> SingleFlightWaiterObservation<'a, Observe>
where
    Observe: Fn(PreflightSingleFlightEvent),
{
    const fn new(observe: &'a Observe) -> Self {
        Self { observe }
    }
}

impl<Observe> Drop for SingleFlightWaiterObservation<'_, Observe>
where
    Observe: Fn(PreflightSingleFlightEvent),
{
    fn drop(&mut self) {
        (self.observe)(PreflightSingleFlightEvent::WaiterFinished);
    }
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::{
        CANONICAL_PREFLIGHT_SCHEMA_V1, CanonicalPreflightDeleteResult,
        CanonicalPreflightDescriptor, CanonicalPreflightKeyV1, CanonicalPreflightLocatorKeyV1,
        CanonicalPreflightLocatorV1, CanonicalPreflightObject, CanonicalPreflightPutResult,
        CanonicalPreflightStore, CanonicalProposalPreflightV1, PreflightCacheRecoveryEvent,
        PreflightCacheResult, PreflightCoordinator, PreflightObserver, SingleFlight,
    };
    use alloy_primitives::B256;
    use anyhow::Result;
    use raiko2_primitives::{L2BlockRange, RaikoError};
    use raiko2_protocol::BlobProofType;
    use raiko2_protocol_shasta::shasta::ShastaEventData;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;
    use tokio::sync::{Barrier, Notify};

    #[derive(Debug, Default)]
    struct TestCanonicalPreflightStore {
        object: Mutex<Option<CanonicalPreflightObject>>,
        deletions: AtomicUsize,
        puts: AtomicUsize,
        fail_delete: std::sync::atomic::AtomicBool,
        replacement_on_delete: Mutex<Option<CanonicalPreflightObject>>,
    }

    #[derive(Debug, Default)]
    struct TestPreflightObserver {
        cache_results: Mutex<Vec<PreflightCacheResult>>,
        recovery_events: Mutex<Vec<PreflightCacheRecoveryEvent>>,
    }

    impl PreflightObserver for TestPreflightObserver {
        fn record_cache_result(&self, result: PreflightCacheResult) {
            self.cache_results
                .lock()
                .expect("test observer lock")
                .push(result);
        }

        fn record_recovery(&self, event: PreflightCacheRecoveryEvent) {
            self.recovery_events
                .lock()
                .expect("test observer lock")
                .push(event);
        }
    }

    #[async_trait::async_trait]
    impl CanonicalPreflightStore for TestCanonicalPreflightStore {
        async fn get_canonical_preflight(
            &self,
            _key: &CanonicalPreflightKeyV1,
        ) -> Result<Option<CanonicalPreflightObject>> {
            Ok(self.object.lock().expect("test store lock").clone())
        }

        async fn put_canonical_preflight_if_absent(
            &self,
            key: &CanonicalPreflightKeyV1,
            bytes: &[u8],
        ) -> Result<CanonicalPreflightPutResult> {
            self.puts.fetch_add(1, Ordering::SeqCst);
            let mut object = self.object.lock().expect("test store lock");
            if let Some(existing) = object.as_ref() {
                return Ok(CanonicalPreflightPutResult::AlreadyExists(existing.clone()));
            }
            let created = CanonicalPreflightObject {
                key_digest: key.digest()?,
                content_hash: "test-content-hash".to_string(),
                generation: Some(1),
                bytes: bytes.to_vec(),
            };
            *object = Some(created.clone());
            Ok(CanonicalPreflightPutResult::Created(created))
        }

        async fn delete_canonical_preflight_exact(
            &self,
            _key: &CanonicalPreflightKeyV1,
            descriptor: &CanonicalPreflightDescriptor,
        ) -> Result<CanonicalPreflightDeleteResult> {
            if self.fail_delete.swap(false, Ordering::SeqCst) {
                anyhow::bail!("injected canonical preflight delete failure");
            }
            let mut object = self.object.lock().expect("test store lock");
            let Some(current) = object.as_ref() else {
                return Ok(CanonicalPreflightDeleteResult::Missing);
            };
            if !current.descriptor().same_storage_version(descriptor) {
                return Ok(CanonicalPreflightDeleteResult::Stale);
            }
            if let Some(replacement) = self
                .replacement_on_delete
                .lock()
                .expect("replacement lock")
                .take()
            {
                *object = Some(replacement);
                return Ok(CanonicalPreflightDeleteResult::Stale);
            }
            object.take();
            self.deletions.fetch_add(1, Ordering::SeqCst);
            Ok(CanonicalPreflightDeleteResult::Removed)
        }
    }

    fn canonical_key() -> CanonicalPreflightKeyV1 {
        CanonicalPreflightKeyV1 {
            schema: CANONICAL_PREFLIGHT_SCHEMA_V1,
            blob_proof_type: BlobProofType::ProofOfEquivalence,
            l1_chain_id: 32_382,
            l2_chain_id: 167_001,
            proposal_id: 42,
            l2_block_range: L2BlockRange {
                start: 100,
                end: 102,
            },
            l1_inclusion_block_number: 77,
            last_anchor_block_number: 99,
            checkpoint: None,
            l1_inclusion_hash: B256::repeat_byte(0x33),
            proposal_event_digest: B256::repeat_byte(0x44),
            chain_rules_fingerprint: B256::repeat_byte(0x55),
        }
    }

    fn locator_key() -> CanonicalPreflightLocatorKeyV1 {
        let key = canonical_key();
        CanonicalPreflightLocatorKeyV1 {
            schema: key.schema,
            blob_proof_type: key.blob_proof_type,
            l1_chain_id: key.l1_chain_id,
            l2_chain_id: key.l2_chain_id,
            proposal_id: key.proposal_id,
            l2_block_range: key.l2_block_range,
            l1_inclusion_block_number: key.l1_inclusion_block_number,
            last_anchor_block_number: key.last_anchor_block_number,
            checkpoint: key.checkpoint,
            chain_rules_fingerprint: key.chain_rules_fingerprint,
        }
    }

    #[tokio::test]
    async fn coordinator_coalesces_concurrent_locator_builds() -> Result<()> {
        let coordinator = Arc::new(PreflightCoordinator::disabled());
        let key = locator_key();
        let calls = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(7));
        let mut requests = Vec::new();
        for _ in 0..6 {
            let coordinator = Arc::clone(&coordinator);
            let request_key = key.clone();
            let calls = Arc::clone(&calls);
            let barrier = Arc::clone(&barrier);
            requests.push(tokio::spawn(async move {
                barrier.wait().await;
                coordinator
                    .locate(request_key.clone(), || {
                        let calls = Arc::clone(&calls);
                        let request_key = request_key.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(20)).await;
                            Ok(CanonicalPreflightLocatorV1 {
                                key: canonical_key(),
                                block_numbers: vec![100, 101],
                                expected_proposal_id: request_key.proposal_id,
                                proposal_event: ShastaEventData::default(),
                            })
                        }
                    })
                    .await
            }));
        }
        barrier.wait().await;

        for request in requests {
            assert_eq!(request.await??.expected_proposal_id, key.proposal_id);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[test]
    fn explicit_bypass_is_observable() {
        let store = Arc::new(TestCanonicalPreflightStore::default());
        let observer = Arc::new(TestPreflightObserver::default());
        let coordinator = PreflightCoordinator::with_observer(store, observer.clone());

        coordinator.record_bypass();

        assert_eq!(
            *observer.cache_results.lock().expect("test observer lock"),
            vec![PreflightCacheResult::Bypass]
        );
    }

    #[tokio::test]
    async fn same_key_runs_one_leader_and_releases_all_waiters() -> Result<()> {
        let single_flight = Arc::new(SingleFlight::<u64, usize>::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(Barrier::new(9));
        let release = Arc::new(Notify::new());
        let mut tasks = Vec::new();

        for _ in 0..8 {
            let single_flight = Arc::clone(&single_flight);
            let calls = Arc::clone(&calls);
            let start = Arc::clone(&start);
            let release = Arc::clone(&release);
            tasks.push(tokio::spawn(async move {
                start.wait().await;
                single_flight
                    .run(7, || {
                        let calls = Arc::clone(&calls);
                        let release = Arc::clone(&release);
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            release.notified().await;
                            Ok::<_, anyhow::Error>(42)
                        }
                    })
                    .await
            }));
        }

        start.wait().await;
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        release.notify_waiters();

        for task in tasks {
            assert_eq!(*task.await??, 42);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(single_flight.in_flight_len(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_leader_allows_waiter_to_become_leader() -> Result<()> {
        let single_flight = Arc::new(SingleFlight::<u64, usize>::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let first_started = Arc::new(Notify::new());
        let never_release = Arc::new(Notify::new());

        let leader = {
            let single_flight = Arc::clone(&single_flight);
            let calls = Arc::clone(&calls);
            let first_started = Arc::clone(&first_started);
            let never_release = Arc::clone(&never_release);
            tokio::spawn(async move {
                single_flight
                    .run(7, || {
                        let call = calls.fetch_add(1, Ordering::SeqCst);
                        let first_started = Arc::clone(&first_started);
                        let never_release = Arc::clone(&never_release);
                        async move {
                            if call == 0 {
                                first_started.notify_one();
                                never_release.notified().await;
                            }
                            Ok::<_, anyhow::Error>(call + 1)
                        }
                    })
                    .await
            })
        };
        first_started.notified().await;

        let waiter = {
            let single_flight = Arc::clone(&single_flight);
            let calls = Arc::clone(&calls);
            tokio::spawn(async move {
                single_flight
                    .run(7, || {
                        let call = calls.fetch_add(1, Ordering::SeqCst);
                        async move { Ok::<_, anyhow::Error>(call + 1) }
                    })
                    .await
            })
        };

        leader.abort();
        let value = tokio::time::timeout(std::time::Duration::from_secs(1), waiter).await???;
        assert_eq!(*value, 2);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(single_flight.in_flight_len(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn failed_build_is_not_negative_cached() -> Result<()> {
        let single_flight = SingleFlight::<u64, usize>::default();
        let calls = AtomicUsize::new(0);

        let error = single_flight
            .run(7, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("injected build failure")
            })
            .await
            .expect_err("first build must fail");
        assert!(error.to_string().contains("injected build failure"));

        let value = single_flight
            .run(7, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, anyhow::Error>(42)
            })
            .await?;
        assert_eq!(*value, 42);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(single_flight.in_flight_len(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_waiters_share_failure_before_a_later_request_retries() -> Result<()> {
        let single_flight = Arc::new(SingleFlight::<u64, usize>::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(Barrier::new(7));
        let release_leader = Arc::new(Notify::new());
        let mut tasks = Vec::new();

        for _ in 0..6 {
            let single_flight = Arc::clone(&single_flight);
            let calls = Arc::clone(&calls);
            let start = Arc::clone(&start);
            let release_leader = Arc::clone(&release_leader);
            tasks.push(tokio::spawn(async move {
                start.wait().await;
                single_flight
                    .run(7, || {
                        let call = calls.fetch_add(1, Ordering::SeqCst);
                        let release_leader = Arc::clone(&release_leader);
                        async move {
                            if call == 0 {
                                release_leader.notified().await;
                            }
                            Err::<usize, _>(anyhow::anyhow!("injected shared failure"))
                        }
                    })
                    .await
            }));
        }

        start.wait().await;
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        release_leader.notify_waiters();

        for task in tasks {
            let error = task.await?.expect_err("shared build must fail");
            assert!(error.to_string().contains("injected shared failure"));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(single_flight.in_flight_len(), 0);

        let value = single_flight
            .run(7, || async {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok::<_, anyhow::Error>(42)
            })
            .await?;
        assert_eq!(*value, 42);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        Ok(())
    }

    #[tokio::test]
    async fn coordinator_publishes_once_then_loads_validated_core() -> Result<()> {
        let store = Arc::new(TestCanonicalPreflightStore::default());
        let coordinator = PreflightCoordinator::new(store);
        let key = canonical_key();
        let builds = AtomicUsize::new(0);
        let validations = AtomicUsize::new(0);

        for _ in 0..2 {
            let core = coordinator
                .canonical(
                    key.clone(),
                    || async {
                        builds.fetch_add(1, Ordering::SeqCst);
                        let mut core = CanonicalProposalPreflightV1::default();
                        core.manifest.proposal_id = 42;
                        Ok(core)
                    },
                    |core| {
                        validations.fetch_add(1, Ordering::SeqCst);
                        if core.manifest.proposal_id != 42 {
                            return Err(RaikoError::Preflight(
                                "unexpected canonical proposal".to_string(),
                            ));
                        }
                        Ok(())
                    },
                )
                .await?;
            assert_eq!(core.manifest.proposal_id, 42);
        }

        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert_eq!(validations.load(Ordering::SeqCst), 2);
        Ok(())
    }

    #[tokio::test]
    async fn same_version_restart_reuses_validated_publication() -> Result<()> {
        let store = Arc::new(TestCanonicalPreflightStore::default());
        let key = canonical_key();
        let mut published = CanonicalProposalPreflightV1::default();
        published.manifest.proposal_id = key.proposal_id;

        PreflightCoordinator::new(store.clone())
            .canonical(key.clone(), || async { Ok(published.clone()) }, |_| Ok(()))
            .await?;

        let restarted = PreflightCoordinator::new(store);
        let builds = AtomicUsize::new(0);
        let loaded = restarted
            .canonical(
                key.clone(),
                || async {
                    builds.fetch_add(1, Ordering::SeqCst);
                    Ok(CanonicalProposalPreflightV1::default())
                },
                |core| {
                    if core.manifest.proposal_id == key.proposal_id {
                        Ok(())
                    } else {
                        Err(RaikoError::Preflight(
                            "unexpected canonical proposal".to_string(),
                        ))
                    }
                },
            )
            .await?;

        assert_eq!(loaded.manifest.proposal_id, key.proposal_id);
        assert_eq!(builds.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn coordinator_invalidates_undecodable_core_before_rebuild() -> Result<()> {
        let store = Arc::new(TestCanonicalPreflightStore {
            object: Mutex::new(Some(CanonicalPreflightObject {
                key_digest: canonical_key().digest()?,
                content_hash: "corrupt".to_string(),
                generation: Some(7),
                bytes: b"not-bincode".to_vec(),
            })),
            ..TestCanonicalPreflightStore::default()
        });
        let coordinator = PreflightCoordinator::new(store.clone());
        let builds = AtomicUsize::new(0);

        let core = coordinator
            .canonical(
                canonical_key(),
                || async {
                    builds.fetch_add(1, Ordering::SeqCst);
                    let mut core = CanonicalProposalPreflightV1::default();
                    core.manifest.proposal_id = 42;
                    Ok(core)
                },
                |_| Ok(()),
            )
            .await?;

        assert_eq!(core.manifest.proposal_id, 42);
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert_eq!(store.deletions.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn coordinator_does_not_publish_failed_canonical_validation() -> Result<()> {
        let store = Arc::new(TestCanonicalPreflightStore::default());
        let coordinator = PreflightCoordinator::new(store.clone());

        let error = coordinator
            .canonical(
                canonical_key(),
                || async { Ok(CanonicalProposalPreflightV1::default()) },
                |_| {
                    Err(RaikoError::Preflight(
                        "injected canonical validation failure".to_string(),
                    ))
                },
            )
            .await
            .expect_err("invalid canonical core must not be published");

        assert!(
            error
                .to_string()
                .contains("injected canonical validation failure")
        );
        assert!(store.object.lock().expect("test store lock").is_none());
        Ok(())
    }

    #[tokio::test]
    async fn coordinator_failed_build_is_not_negative_cached() -> Result<()> {
        let store = Arc::new(TestCanonicalPreflightStore::default());
        let coordinator = PreflightCoordinator::new(store.clone());
        let calls = AtomicUsize::new(0);

        let error = coordinator
            .canonical(
                canonical_key(),
                || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Err(RaikoError::Preflight("injected build failure".to_string()))
                },
                |_| Ok(()),
            )
            .await
            .expect_err("first build must fail");
        assert!(error.to_string().contains("injected build failure"));

        let core = coordinator
            .canonical(
                canonical_key(),
                || async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let mut core = CanonicalProposalPreflightV1::default();
                    core.manifest.proposal_id = 42;
                    Ok(core)
                },
                |_| Ok(()),
            )
            .await?;
        assert_eq!(core.manifest.proposal_id, 42);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(store.puts.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    async fn delete_failure_rebuilds_without_cache_publication() -> Result<()> {
        let key = canonical_key();
        let stale = CanonicalPreflightObject {
            key_digest: key.digest()?,
            content_hash: "corrupt".to_string(),
            generation: Some(7),
            bytes: b"not-bincode".to_vec(),
        };
        let store = Arc::new(TestCanonicalPreflightStore {
            object: Mutex::new(Some(stale.clone())),
            fail_delete: std::sync::atomic::AtomicBool::new(true),
            ..TestCanonicalPreflightStore::default()
        });
        let observer = Arc::new(TestPreflightObserver::default());
        let coordinator = PreflightCoordinator::with_observer(store.clone(), observer.clone());

        let core = coordinator
            .canonical(
                key,
                || async {
                    let mut core = CanonicalProposalPreflightV1::default();
                    core.manifest.proposal_id = 42;
                    Ok(core)
                },
                |_| Ok(()),
            )
            .await?;

        assert_eq!(core.manifest.proposal_id, 42);
        assert_eq!(store.puts.load(Ordering::SeqCst), 0);
        assert_eq!(*store.object.lock().expect("test store lock"), Some(stale));
        assert_eq!(
            *observer.recovery_events.lock().expect("test observer lock"),
            vec![
                PreflightCacheRecoveryEvent::InvalidEntry,
                PreflightCacheRecoveryEvent::ExactDeleteFailure,
                PreflightCacheRecoveryEvent::Rebuild,
                PreflightCacheRecoveryEvent::UncachedFallback,
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn stale_delete_reloads_and_uses_current_winner_once() -> Result<()> {
        let key = canonical_key();
        let stale = CanonicalPreflightObject {
            key_digest: key.digest()?,
            content_hash: "corrupt".to_string(),
            generation: Some(7),
            bytes: b"not-bincode".to_vec(),
        };
        let mut winner_core = CanonicalProposalPreflightV1::default();
        winner_core.manifest.proposal_id = 42;
        let winner = CanonicalPreflightObject {
            key_digest: key.digest()?,
            content_hash: "winner".to_string(),
            generation: Some(8),
            bytes: bincode::serialize(&winner_core)?,
        };
        let store = Arc::new(TestCanonicalPreflightStore {
            object: Mutex::new(Some(stale)),
            replacement_on_delete: Mutex::new(Some(winner)),
            ..TestCanonicalPreflightStore::default()
        });
        let coordinator = PreflightCoordinator::new(store.clone());
        let builds = AtomicUsize::new(0);

        let core = coordinator
            .canonical(
                key,
                || async {
                    builds.fetch_add(1, Ordering::SeqCst);
                    Ok(CanonicalProposalPreflightV1::default())
                },
                |core| {
                    if core.manifest.proposal_id == 42 {
                        Ok(())
                    } else {
                        Err(RaikoError::Preflight("unexpected proposal".to_string()))
                    }
                },
            )
            .await?;

        assert_eq!(core.manifest.proposal_id, 42);
        assert_eq!(builds.load(Ordering::SeqCst), 0);
        assert_eq!(store.puts.load(Ordering::SeqCst), 0);
        Ok(())
    }

    #[tokio::test]
    async fn coordinator_invalidates_semantically_invalid_core_before_rebuild() -> Result<()> {
        let key = canonical_key();
        let mut stale = CanonicalProposalPreflightV1::default();
        stale.manifest.proposal_id = 7;
        let store = Arc::new(TestCanonicalPreflightStore {
            object: Mutex::new(Some(CanonicalPreflightObject {
                key_digest: key.digest()?,
                content_hash: "stale".to_string(),
                generation: Some(7),
                bytes: bincode::serialize(&stale)?,
            })),
            ..TestCanonicalPreflightStore::default()
        });
        let coordinator = PreflightCoordinator::new(store.clone());
        let builds = AtomicUsize::new(0);

        let core = coordinator
            .canonical(
                key,
                || async {
                    builds.fetch_add(1, Ordering::SeqCst);
                    let mut core = CanonicalProposalPreflightV1::default();
                    core.manifest.proposal_id = 42;
                    Ok(core)
                },
                |core| {
                    if core.manifest.proposal_id != 42 {
                        return Err(RaikoError::Preflight(
                            "canonical proposal mismatch".to_string(),
                        ));
                    }
                    Ok(())
                },
            )
            .await?;

        assert_eq!(core.manifest.proposal_id, 42);
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert_eq!(store.deletions.load(Ordering::SeqCst), 1);
        Ok(())
    }
}
