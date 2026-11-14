//! The CLI owns the identity of the executable shared by all subcommands,
//! including LSP. Missing build-script fields are compile errors here.
use verum_error::crash::BuildIdentity;

pub const BUILD_IDENTITY: BuildIdentity = BuildIdentity {
    profile: env!("VERUM_BUILD_PROFILE"),
    target: env!("VERUM_BUILD_TARGET"),
    host: env!("VERUM_BUILD_HOST"),
    rustc: env!("VERUM_BUILD_RUSTC"),
    git_sha: env!("VERUM_BUILD_GIT_SHA"),
    git_dirty: env!("VERUM_BUILD_GIT_DIRTY"),
    timestamp: env!("VERUM_BUILD_TIMESTAMP"),
};
