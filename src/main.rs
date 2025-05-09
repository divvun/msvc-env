use msvc_env::MsvcEnv;
use std::{
    env,
    path::{Path, Prefix},
};

fn unixify_path(path: &Path) -> String {
    let p = path.components()
        .filter_map(|c| Some(match c {
            std::path::Component::Prefix(prefix_component) => match prefix_component.kind() {
                Prefix::Disk(os_str) => format!("/{}", os_str as char).to_lowercase(),
                _ => format!("{}", prefix_component.as_os_str().to_str().unwrap()),
            },
            std::path::Component::RootDir => return None,
            std::path::Component::CurDir => ".".to_string(),
            std::path::Component::ParentDir => "..".to_string(),
            std::path::Component::Normal(os_str) => os_str.to_str().unwrap().replace(" ", "\\ "),
        }))
        .collect::<Vec<_>>();
    // eprintln!("P: {:?}", p);
    p.join("/")
}

fn unixify_path_env(path: &str) -> String {
    path.split(";")
        .map(|p| Path::new(p))
        .map(unixify_path)
        .collect::<Vec<_>>()
        .join(":")
}

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
            if key.to_uppercase() == "PATH" {
                println!("export OLD_PATH=\"$PATH\"");
                println!("export PATH={:?}", unixify_path_env(&value));
            } else {
                println!("export {}={:?}", key, value);
            }
        }
    }
}
