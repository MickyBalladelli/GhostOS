use ghostos_rustd::{
    BuildPolicy, BuildRequest, CompilerService, NetworkPolicy, Profile, ResourceLimits, Target,
    Text, MAX_BINARY_BYTES, MAX_FEATURES, MAX_PATH_BYTES,
};
use ghostos_system_model::ContentId;

fn main() {
    let request = BuildRequest {
        source_root: Text::<MAX_PATH_BYTES>::new("/system/sources/demo").expect("source root"),
        manifest: Text::<MAX_PATH_BYTES>::new("/system/sources/demo/Cargo.toml")
            .expect("manifest"),
        binary: Text::<MAX_BINARY_BYTES>::new("demo").expect("binary name"),
        target: Target::X86_64,
        profile: Profile::Release,
        locked: true,
        network: NetworkPolicy::Denied,
        limits: ResourceLimits::DEFAULT,
        features: [None; MAX_FEATURES],
    };

    let mut compiler = CompilerService::<4, 4>::new(BuildPolicy::OFFLINE);
    compiler
        .set_toolchain_identity(ContentId::from_bytes([9; 32]))
        .expect("toolchain identity");
    let job = compiler.submit(request).expect("job is accepted");
    assert_eq!(compiler.status(job).expect("job exists").request, request);
    println!("queued compiler job {}", job.raw());
}
