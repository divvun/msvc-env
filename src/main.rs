use msvc_env::MsvcEnv;

fn main() {
    unsafe { std::env::set_var("RUST_LOG", "trace") };
    tracing_subscriber::fmt::init();
    let env = MsvcEnv.environment(msvc_env::MsvcArch::X64).unwrap();
    let env_vars = env.vars;
    for (key, value) in env_vars {
        println!("{} = {}", key, value);
    }
}
