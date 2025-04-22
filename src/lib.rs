use std::collections::HashMap;
use std::fs;
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use tempfile;
use thiserror::Error;
use tracing::Level;

const VSWHERE_URL: &str =
    "https://github.com/microsoft/vswhere/releases/download/3.1.7/vswhere.exe";

static VSWHERE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static ENV_CACHE: OnceLock<Mutex<HashMap<MsvcArch, MsvcEnvironment>>> = OnceLock::new();

/// Extension trait for Command to add MSVC environment variables
pub trait CommandExt {
    /// Configures the command to use the MSVC environment for the specified architecture
    fn msvc_env(&mut self, arch: MsvcArch) -> Result<&mut Command, MsvcEnvError>;
}

impl CommandExt for Command {
    fn msvc_env(&mut self, arch: MsvcArch) -> Result<&mut Command, MsvcEnvError> {
        let msvc_env = MsvcEnv::new();
        println!("Getting environment for {:?}", arch);
        let env = msvc_env.environment(arch)?;
        println!("Environment: {:?}", env);

        self.envs(&env.vars);
        println!("Environment set");
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MsvcArch {
    X86,
    X64,
    Arm,
    Arm64,
    All,
}

impl MsvcArch {
    fn vcvars_arg(&self) -> &'static str {
        match self {
            MsvcArch::X86 => "x86",
            MsvcArch::X64 => "x64",
            MsvcArch::Arm => "arm",
            MsvcArch::Arm64 => "arm64",
            MsvcArch::All => "all",
        }
    }

    fn bat_filename(&self) -> &'static str {
        match self {
            MsvcArch::X64 => "vcvars64.bat",
            MsvcArch::Arm => "vcvarsamd64_arm.bat",
            MsvcArch::Arm64 => "vcvarsamd64_arm64.bat",
            MsvcArch::X86 => "vcvarsamd64_x86.bat",
            MsvcArch::All => "vcvarsall.bat",
        }
    }

    /// Checks if this architecture's environment is valid by attempting to run a simple MSVC command
    pub fn is_valid_environment(&self) -> bool {
        let _env = match MsvcEnv::new().environment(*self) {
            Ok(env) => env,
            Err(_) => return false,
        };

        let mut cmd = Command::new("cl");
        cmd.msvc_env(*self).ok();

        match cmd.arg("/?").output() {
            Ok(output) => output.status.success(),
            Err(_) => false,
        }
    }
}

impl std::fmt::Display for MsvcArch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.vcvars_arg())
    }
}

#[derive(Error, Debug)]
pub enum MsvcEnvError {
    #[error("Failed to create cache directory: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Failed to download vswhere: {0}")]
    DownloadError(String),
    #[error("Failed to execute vswhere: {0}")]
    VswhereError(String),
    #[error("No Visual Studio installation found")]
    NoVisualStudio,
    #[error("Visual Studio installation found but {0} architecture is not supported (missing {1})")]
    ArchNotSupported(MsvcArch, String),
    #[error("Failed to execute vcvars: {0}")]
    VcvarsError(String),
    #[error("Failed to parse vcvars output: {0}")]
    ParseError(String),
}

/// Represents the environment variables needed for MSVC
#[derive(Debug, Clone)]
pub struct MsvcEnvironment {
    /// All environment variables from vcvars
    pub vars: HashMap<String, String>,
}

pub struct MsvcEnv;

const VSWHERE_PATH: &str = "target/msvc-env-cache";
const VSWHERE_EXE: &str = "vswhere.exe";

impl MsvcEnv {
    pub fn new() -> Self {
        Self
    }

    fn download_vswhere(&self) -> Result<(), MsvcEnvError> {
        let lock = VSWHERE_LOCK.get_or_init(|| Mutex::new(()));
        let _lock = lock
            .lock()
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Mutex poisoned"))?;

        fs::create_dir_all(VSWHERE_PATH)?;

        let vswhere_path = PathBuf::from(VSWHERE_PATH).join(VSWHERE_EXE);

        // Download vswhere if it doesn't exist
        if !vswhere_path.exists() {
            tracing::trace!("Downloading vswhere to {}", vswhere_path.display());
            let response = ureq::get(VSWHERE_URL)
                .call()
                .map_err(|e| MsvcEnvError::DownloadError(e.to_string()))?;

            let (_, body) = response.into_parts();
            let mut file = fs::File::create(&vswhere_path)?;
            let mut reader = body.into_reader();
            std::io::copy(&mut reader, &mut file)?;
        }

        Ok(())
    }

    pub fn find_visual_studio(&self) -> Result<PathBuf, MsvcEnvError> {
        self.download_vswhere()?;
        let vswhere_path = PathBuf::from(VSWHERE_PATH).join(VSWHERE_EXE);

        tracing::trace!("Running vswhere to find Visual Studio");
        let output = Command::new(&vswhere_path)
            .args(&["-latest", "-products", "*", "-property", "installationPath"])
            .output()
            .map_err(|e| MsvcEnvError::VswhereError(e.to_string()))?;

        if !output.status.success() {
            return Err(MsvcEnvError::VswhereError(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }

        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path.is_empty() {
            return Err(MsvcEnvError::NoVisualStudio);
        }

        let path = PathBuf::from(path);
        tracing::trace!("Found Visual Studio at {}", path.display());
        Ok(path)
    }

    pub fn vc_path(&self, arch: MsvcArch) -> Result<PathBuf, MsvcEnvError> {
        let vs_path = self.find_visual_studio()?;
        let vc_path = vs_path.join("VC");

        // Check if the specific bat file exists
        let bat_path = vc_path
            .join("Auxiliary")
            .join("Build")
            .join(arch.bat_filename());

        if !bat_path.exists() {
            tracing::trace!(
                "Architecture {} not supported (missing {})",
                arch,
                arch.bat_filename()
            );
            return Err(MsvcEnvError::ArchNotSupported(
                arch,
                arch.bat_filename().to_string(),
            ));
        }

        tracing::trace!("Found VC path at {}", vc_path.display());
        Ok(vc_path)
    }

    pub fn vcvars_path(&self, arch: MsvcArch) -> Result<PathBuf, MsvcEnvError> {
        let vc_path = self.vc_path(arch)?;
        let vcvars_path = vc_path
            .join("Auxiliary")
            .join("Build")
            .join(arch.bat_filename());

        if !vcvars_path.exists() {
            return Err(MsvcEnvError::NoVisualStudio);
        }

        tracing::trace!("Found vcvars at {}", vcvars_path.display());
        Ok(vcvars_path)
    }

    /// Lists all .bat files in the Auxiliary/Build directory
    pub fn list_bat_files(&self) -> Result<Vec<PathBuf>, MsvcEnvError> {
        let vs_path = self.find_visual_studio()?;
        let build_dir = vs_path.join("VC").join("Auxiliary").join("Build");

        if !build_dir.exists() {
            return Err(MsvcEnvError::NoVisualStudio);
        }

        let mut bat_files = Vec::new();
        for entry in fs::read_dir(build_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "bat") {
                bat_files.push(path);
            }
        }

        Ok(bat_files)
    }

    /// Gets the environment variables for the specified architecture by running vcvarsall.bat
    /// Returns a struct containing all environment variables set by vcvars
    pub fn environment(&self, arch: MsvcArch) -> Result<MsvcEnvironment, MsvcEnvError> {
        // Get or initialize the cache
        let cache = ENV_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        let mut cache = cache.lock().unwrap();

        // Check if we have a cached environment for this architecture
        if let Some(env) = cache.get(&arch) {
            tracing::trace!("Using cached environment for {:?}", arch);
            return Ok(env.clone());
        }

        tracing::trace!("Not cached, getting environment");
        // If not cached, get the environment
        let vcvars_path = self.vcvars_path(arch)?;
        let new_env = self.vcvars_environment(&vcvars_path, arch)?;
        let env = MsvcEnvironment { vars: new_env };

        // Cache the environment
        cache.insert(arch, env.clone());

        Ok(env)
    }

    /// Gets the environment variables after running vcvars
    fn vcvars_environment(
        &self,
        vcvars_path: &Path,
        arch: MsvcArch,
    ) -> Result<HashMap<String, String>, MsvcEnvError> {
        let temp_dir = tempfile::tempdir().map_err(|e| MsvcEnvError::IoError(e.into()))?;
        let temp_bat = temp_dir.path().join("getenv.bat");

        let batch_content = format!(
            "@echo off\r\n\
            call \"{}\" {}\r\n\
            if errorlevel 1 exit /b %errorlevel%\r\n\
            set\r\n",
            vcvars_path.display(),
            arch.vcvars_arg(),
        );

        tracing::trace!("vcvars_path: {}", vcvars_path.display());
        tracing::trace!("arch: {:?}", arch);
        tracing::trace!("batch_content: {}", batch_content);
        tracing::trace!("temp_bat: {}", temp_bat.display());

        fs::write(&temp_bat, batch_content)?;

        let output = Command::new("cmd")
            .args(&["/C", temp_bat.to_str().unwrap()])
            .output()
            .map_err(|e| MsvcEnvError::VcvarsError(e.to_string()))?;

        if !output.status.success() {
            return Err(MsvcEnvError::VcvarsError(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }

        self.parse_environment_output(&output.stdout)
    }

    /// Parses the output of the 'set' command into a HashMap
    fn parse_environment_output(
        &self,
        output: &[u8],
    ) -> Result<HashMap<String, String>, MsvcEnvError> {
        let output_str = String::from_utf8_lossy(output);
        let mut env = HashMap::new();

        for line in output_str.lines() {
            if let Some((key, value)) = line.split_once('=') {
                env.insert(key.to_string(), value.to_string());
            }
        }

        tracing::trace!("Parsed {} environment variables", env.len());
        Ok(env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn cleanup_cache() {
        let cache_dir = PathBuf::from("target/msvc-env-cache");
        if cache_dir.exists() {
            // Try to remove the file first
            let vswhere_path = cache_dir.join("vswhere.exe");
            if vswhere_path.exists() {
                let _ = fs::remove_file(&vswhere_path);
            }
            // Then remove the directory
            let _ = fs::remove_dir_all(&cache_dir);
        }
    }

    #[test]
    fn test_vswhere_download() {
        cleanup_cache();

        // Create new instance which should download vswhere
        let msvc_env = MsvcEnv::new();
        msvc_env.download_vswhere().unwrap();
    }

    #[test]
    fn test_find_visual_studio() {
        cleanup_cache();
        let msvc_env = MsvcEnv::new();

        // Test each architecture
        for arch in [
            MsvcArch::X86,
            MsvcArch::X64,
            MsvcArch::Arm,
            MsvcArch::Arm64,
            MsvcArch::All,
        ] {
            println!("Testing Visual Studio detection for {:?}", arch);
            match msvc_env.find_visual_studio() {
                Ok(path) => {
                    assert!(path.exists());
                    assert!(path.is_dir());
                    println!("Found Visual Studio at: {}", path.display());
                }
                Err(MsvcEnvError::NoVisualStudio) => {
                    println!(
                        "No Visual Studio installation found for {:?} - this is expected if VS is not installed",
                        arch
                    );
                }
                Err(e) => panic!("Unexpected error for {:?}: {}", arch, e),
            }
        }
    }

    #[test]
    fn test_vc_path() {
        cleanup_cache();
        let msvc_env = MsvcEnv::new();

        // Test each architecture
        for arch in [
            MsvcArch::X86,
            MsvcArch::X64,
            MsvcArch::Arm,
            MsvcArch::Arm64,
            MsvcArch::All,
        ] {
            println!("Testing VC path detection for {:?}", arch);
            match msvc_env.vc_path(arch) {
                Ok(path) => {
                    assert!(path.exists());
                    assert!(path.is_dir());
                    println!("Found VC path at: {}", path.display());
                }
                Err(MsvcEnvError::NoVisualStudio) => {
                    println!(
                        "No Visual Studio installation found for {:?} - this is expected if VS is not installed",
                        arch
                    );
                }
                Err(MsvcEnvError::ArchNotSupported(arch, _)) => {
                    println!(
                        "Arch {:?} not supported - this is expected if VS is not installed",
                        arch
                    );
                }
                Err(e) => panic!("Unexpected error for {:?}: {}", arch, e),
            }
        }
    }

    #[test]
    fn test_environment() {
        cleanup_cache();
        let msvc_env = MsvcEnv::new();

        // Test each architecture
        for arch in [
            MsvcArch::X86,
            MsvcArch::X64,
            MsvcArch::Arm,
            MsvcArch::Arm64,
            MsvcArch::All,
        ] {
            println!("Testing environment setup for {:?}", arch);
            match msvc_env.environment(arch) {
                Ok(env) => {
                    println!("Environment variables found: {}", env.vars.len());
                    // Print some key variables for debugging
                    for key in ["PATH", "INCLUDE", "LIB", "Platform", "VSCMD_ARG_TGT_ARCH"].iter() {
                        if let Some(value) = env.vars.get(*key) {
                            println!("{} = {}", key, value);
                        }
                    }
                }
                Err(MsvcEnvError::NoVisualStudio) => {
                    println!(
                        "No Visual Studio installation found for {:?} - this is expected if VS is not installed",
                        arch
                    );
                }
                Err(MsvcEnvError::ArchNotSupported(arch, _)) => {
                    println!(
                        "Arch {:?} not supported - this is expected if VS is not installed",
                        arch
                    );
                }
                Err(MsvcEnvError::VcvarsError(e)) => {
                    println!("Vcvars error: {}", e);
                }
                Err(e) => panic!("Unexpected error for {:?}: {}", arch, e),
            }
        }
    }

    #[test]
    fn test_command_ext() {
        cleanup_cache();

        // Test each architecture
        for arch in [
            MsvcArch::X86,
            MsvcArch::X64,
            MsvcArch::Arm,
            MsvcArch::Arm64,
            MsvcArch::All,
        ] {
            println!("Testing CommandExt for {:?}", arch);
            // Create a command and configure it with MSVC environment
            let mut cmd = Command::new("cl");
            match cmd.msvc_env(arch) {
                Ok(_) => {
                    println!(
                        "Successfully configured command with MSVC environment for {:?}",
                        arch
                    );
                }
                Err(MsvcEnvError::NoVisualStudio) => {
                    println!(
                        "No Visual Studio installation found for {:?} - this is expected if VS is not installed",
                        arch
                    );
                }
                Err(MsvcEnvError::ArchNotSupported(arch, _)) => {
                    println!(
                        "Arch {:?} not supported - this is expected if VS is not installed",
                        arch
                    );
                }
                Err(e) => panic!("Unexpected error for {:?}: {}", arch, e),
            }
        }
    }

    #[test]
    fn test_list_bat_files() {
        cleanup_cache();
        let msvc_env = MsvcEnv::new();

        match msvc_env.list_bat_files() {
            Ok(files) => {
                println!("Found .bat files:");
                for file in files {
                    println!("  {}", file.display());
                }
            }
            Err(e) => println!("Error listing .bat files: {}", e),
        }
    }

    #[test]
    fn test_msvc_executables() {
        cleanup_cache();
        let msvc_env = MsvcEnv::new();

        // Test each architecture
        for arch in [MsvcArch::X86, MsvcArch::X64, MsvcArch::Arm64, MsvcArch::All] {
            println!("\nTesting MSVC executables for {:?}", arch);

            // Get the VC path
            match msvc_env.vc_path(arch) {
                Ok(vc_path) => {
                    println!("Found VC path at: {}", vc_path.display());

                    // Get the environment
                    match msvc_env.environment(arch) {
                        Ok(env) => {
                            println!("Environment variables found: {}", env.vars.len());

                            // Test each executable
                            for exe in ["cl", "link", "mc", "rc", "lib"] {
                                println!("\nTesting {}:", exe);
                                let mut cmd = Command::new(exe);
                                match cmd.msvc_env(arch) {
                                    Ok(_) => {
                                        // Run with /? to get help output
                                        let output = cmd.output().unwrap();
                                        println!(
                                            "{} output:\n{}",
                                            exe,
                                            String::from_utf8_lossy(&output.stdout)
                                        );
                                        println!(
                                            "{} stderr:\n{}",
                                            exe,
                                            String::from_utf8_lossy(&output.stderr)
                                        );
                                    }
                                    Err(MsvcEnvError::NoVisualStudio) => {
                                        println!("Visual Studio not found - skipping test");
                                    }
                                    Err(MsvcEnvError::ArchNotSupported(_, _)) => {
                                        println!("Architecture not supported - skipping test");
                                    }
                                    Err(e) => panic!("Unexpected error for {}: {}", exe, e),
                                }
                            }
                        }
                        Err(MsvcEnvError::NoVisualStudio) => {
                            println!("No Visual Studio installation found - skipping test");
                        }
                        Err(MsvcEnvError::ArchNotSupported(_, _)) => {
                            println!("Architecture not supported - skipping test");
                        }
                        Err(e) => panic!("Unexpected error: {}", e),
                    }
                }
                Err(MsvcEnvError::NoVisualStudio) => {
                    println!("No Visual Studio installation found - skipping test");
                }
                Err(MsvcEnvError::ArchNotSupported(_, _)) => {
                    println!("Architecture not supported - skipping test");
                }
                Err(e) => panic!("Unexpected error: {}", e),
            }
        }
    }
}
