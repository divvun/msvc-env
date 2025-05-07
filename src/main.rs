use msvc_env::MsvcEnv;
use std::env;

fn main() {
    tracing_subscriber::fmt::init();

    let args = env::args().collect::<Vec<_>>();
    let flags = args
        .iter()
        .filter(|arg| arg.starts_with("-"))
        .map(|x| &**x)
        .collect::<Vec<_>>();
    let arch = args
        .iter()
        .skip(1)
        .filter(|arg| !arg.starts_with("-"))
        .next();

    // Get architecture from command line args or default to X64
    let arch = arch
        .map(|arg| match arg.to_lowercase().as_str() {
            "x64" => msvc_env::MsvcArch::X64,
            "x86" => msvc_env::MsvcArch::X86,
            "arm" => msvc_env::MsvcArch::Arm,
            "arm64" => msvc_env::MsvcArch::Arm64,
            "all" => msvc_env::MsvcArch::All,
            _ => {
                eprintln!(
                    "Invalid architecture: {}. Supported architectures: x64, x86, arm, arm64, all",
                    arg
                );
                std::process::exit(1);
            }
        })
        .unwrap_or(msvc_env::MsvcArch::X64);

    let env = MsvcEnv.environment(arch).unwrap();
    let env_vars = env.vars;

    if flags.contains(&"-v") {
        eprintln!("Environment: {:#?}", env_vars);
    }

    let is_shell = flags.contains(&"--sh");

    for (key, value) in env_vars {
        if !is_shell {
            println!(
                "${{env:{}}}={}",
                key,
                format!("{:?}", value).replace(r"\\", r"\")
            );
        } else if !(key.contains("(") || key.contains(")")) {
            println!("export {:?}={:?}", key, value);
        }
    }
}
