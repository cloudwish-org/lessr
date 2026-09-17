//! omp. Confidence: unverified as an agent of its own, because it is not one:
//! omp runs on the pi harness, and `lessr init --agent pi` installs the
//! extension it will use. The entry exists so that a user who installed omp
//! sees omp in `lessr init` rather than being left to work that out.

use crate::agent::AgentId;
use crate::agents::Unverified;

/// See the module docs.
pub(crate) static ADAPTER: Unverified = Unverified {
    id: AgentId::Omp,
    probes: &[".omp", ".config/omp"],
    note: "omp runs on the pi harness, so there is nothing separate to write here: run\n\
           `lessr init --agent pi`, which installs the extension omp will use.",
};
