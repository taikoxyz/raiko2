// Copyright 2025 RISC Zero, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// The upstream RPC provider URL to forward requests to.
    #[arg(long, env)]
    pub rpc_url: String,

    /// The network address and port to bind the server to.
    #[arg(long, default_value = "127.0.0.1:8545")]
    pub bind_address: String,

    /// The initial backoff in milliseconds.
    #[clap(long, default_value_t = 500)]
    pub rpc_retry_backoff: u64,

    /// The number of allowed Compute Units per second.
    #[clap(long, default_value_t = 1000)]
    pub rpc_retry_cu: u64,
}
