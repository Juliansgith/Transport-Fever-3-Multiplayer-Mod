//! Builds in the commit, the build time and, on Windows, the version
//! resource (`tpf3mp-buildinfo`).

fn main() {
    tpf3mp_buildinfo::emit(
        tpf3mp_buildinfo::Artifact::Program,
        "TPF3-MP agent",
        "tpf3mp-agent.exe",
    );
}
