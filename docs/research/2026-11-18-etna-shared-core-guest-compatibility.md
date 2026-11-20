# Historical Etna shared-core guest compatibility

This is historical CPU compatibility evidence, not a current build target or a
production proof. The diagnostic source, fixtures, runners and measured results
are pinned at [Raiko2 a29b53c4](https://github.com/taikoxyz/raiko2/tree/a29b53c462b69035ef2acfb5785d78ba20067605/crates/etna-guest-compat).
The [original report](https://github.com/taikoxyz/raiko2/blob/a29b53c462b69035ef2acfb5785d78ba20067605/crates/etna-guest-compat/README.md)
records RISC0 3.0.5 and SP1 6.8.1 CPU execution under the then-existing guest
crypto patches and comparison with native and independent stock-oracle journals.
The matching journal was 684 bytes; no genuine proof or L1 acceptance was tested.

The later [624331f7 cleanup](https://github.com/taikoxyz/raiko2/commit/624331f7)
removed unused generator fields from an included fixture. Native execution after
that cleanup still matched the journal, but the earlier guest ELF hashes and
cycle counts were not remeasured. They do not describe a current build.

The executable diagnostics have been removed from the active branch instead of
keeping them in production guest workspaces. Both production guest manifests and
locks retain their base production graph; diagnostics no longer add a second
Etna source pin or force production resolution of the diagnostic dependencies.
Reproduce the historical experiment from its pinned commit when needed. Current
Etna integration must separately satisfy the P00-P08 plan, production provenance,
canonical guest builds, full range checks and genuine proof acceptance.
