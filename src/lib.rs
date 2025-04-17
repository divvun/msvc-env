use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;
use std::collections::HashMap;
use thiserror::Error;

const VSWHERE_URL: &str = "https://github.com/microsoft/vswhere/releases/download/3.1.1/vswhere.exe";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsvcArch {
    X86,
    X64,
    Arm,
    Arm64,
}

impl MsvcArch {
    fn vcvars_arg(&self) -> &'static str {
        match self {
            MsvcArch::X86 => "x86",
            MsvcArch::X64 => "x64",
            MsvcArch::Arm => "arm",
            MsvcArch::Arm64 => "arm64",
        }
    }
}

#[derive(Error, Debug)]
pub enum MsvcEnvError {
    #[error("Failed to create cache directory: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Failed to download vswhere: {0}")]
    DownloadError(#[from] reqwest::Error),
    #[error("Failed to execute vswhere: {0}")]
    VswhereError(String),
    #[error("No Visual Studio installation found")]
    NoVisualStudio,
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

pub struct MsvcEnv {
    vswhere_path: PathBuf,
}

impl MsvcEnv {
    pub fn new() -> Result<Self, MsvcEnvError> {
        // Create a cache directory in target
        let tardir = PathBuf::from("target");
        let cache_dir = tardir.join("msvc-env-cache");
        fs::create_dir_all(&cache_dir)?;

        let vswhere_path = cache_dir.join("vswhere.exe");
        
        // Download vswhere if it doesn't exist
        if !vswhere_path.exists() {
            let response = reqwest::blocking::get(VSWHERE_URL)?;
            let mut file = fs::File::create(&vswhere_path)?;
            let content = response.bytes()?;
            std::io::copy(&mut content.as_ref(), &mut file)?;
        }

        Ok(Self { vswhere_path })
    }

    pub fn find_visual_studio(&self) -> Result<PathBuf, MsvcEnvError> {
        let output = Command::new(&self.vswhere_path)
            .args(&["-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"])
            .output()
            .map_err(|e| MsvcEnvError::VswhereError(e.to_string()))?;

        if !output.status.success() {
            return Err(MsvcEnvError::VswhereError(String::from_utf8_lossy(&output.stderr).into_owned()));
        }

        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if path.is_empty() {
            return Err(MsvcEnvError::NoVisualStudio);
        }

        Ok(PathBuf::from(path))
    }

    pub fn vc_path(&self) -> Result<PathBuf, MsvcEnvError> {
        let vs_path = self.find_visual_studio()?;
        let vc_path = vs_path.join("VC");
        
        if !vc_path.exists() {
            return Err(MsvcEnvError::NoVisualStudio);
        }

        Ok(vc_path)
    }

    pub fn vcvars_path(&self) -> Result<PathBuf, MsvcEnvError> {
        let vc_path = self.vc_path()?;
        let vcvars_path = vc_path.join("Auxiliary").join("Build").join("vcvarsall.bat");
        
        if !vcvars_path.exists() {
            return Err(MsvcEnvError::NoVisualStudio);
        }

        Ok(vcvars_path)
    }

    /// Gets the environment variables for the specified architecture by running vcvarsall.bat
    /// Returns a struct containing all environment variables set by vcvars
    pub fn environment(&self, arch: MsvcArch) -> Result<MsvcEnvironment, MsvcEnvError> {
        let vcvars_path = self.vcvars_path()?;
        
        // Then get the environment after running vcvars
        let new_env = self.vcvars_environment(&vcvars_path, arch)?;
        
        // Create the final environment with all variables
        Ok(MsvcEnvironment { vars: new_env })
    }

    /// Gets the environment variables after running vcvars
    fn vcvars_environment(&self, vcvars_path: &Path, arch: MsvcArch) -> Result<HashMap<String, String>, MsvcEnvError> {
        // Create a batch file that will run vcvars and output the environment
        let temp_dir = tempfile::tempdir().map_err(|e| MsvcEnvError::IoError(e.into()))?;
        let temp_bat = temp_dir.path().join("getenv.bat");
        
        let batch_content = format!(
            "@echo off\r\n\
            call \"{}\" {} > nul 2>&1\r\n\
            if errorlevel 1 exit /b %errorlevel%\r\n\
            set\r\n",
            vcvars_path.display(),
            arch.vcvars_arg(),
        );
        
        fs::write(&temp_bat, batch_content)?;

        let output = Command::new("cmd")
            .args(&["/C", temp_bat.to_str().unwrap()])
            .output()
            .map_err(|e| MsvcEnvError::VcvarsError(e.to_string()))?;

        if !output.status.success() {
            return Err(MsvcEnvError::VcvarsError(String::from_utf8_lossy(&output.stderr).into_owned()));
        }

        self.parse_environment_output(&output.stdout)
    }

    /// Parses the output of the 'set' command into a HashMap
    fn parse_environment_output(&self, output: &[u8]) -> Result<HashMap<String, String>, MsvcEnvError> {
        let output_str = String::from_utf8_lossy(output);
        let mut env = HashMap::new();

        for line in output_str.lines() {
            if let Some((key, value)) = line.split_once('=') {
                env.insert(key.to_string(), value.to_string());
            }
        }

        Ok(env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use serial_test::serial;

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
    #[serial]
    fn test_vswhere_download() {
        cleanup_cache();

        // Create new instance which should download vswhere
        let msvc_env = MsvcEnv::new().unwrap();
        
        // Verify vswhere was downloaded
        assert!(msvc_env.vswhere_path.exists());
        assert!(msvc_env.vswhere_path.is_file());
    }

    #[test]
    #[serial]
    fn test_find_visual_studio() {
        cleanup_cache();
        let msvc_env = MsvcEnv::new().unwrap();
        
        // This test will only pass if Visual Studio is installed
        match msvc_env.find_visual_studio() {
            Ok(path) => {
                assert!(path.exists());
                assert!(path.is_dir());
                println!("Found Visual Studio at: {}", path.display());
            }
            Err(MsvcEnvError::NoVisualStudio) => {
                println!("No Visual Studio installation found - this is expected if VS is not installed");
            }
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }

    #[test]
    #[serial]
    fn test_vc_path() {
        cleanup_cache();
        let msvc_env = MsvcEnv::new().unwrap();
        
        // This test will only pass if Visual Studio with VC tools is installed
        match msvc_env.vc_path() {
            Ok(path) => {
                assert!(path.exists());
                assert!(path.is_dir());
                println!("Found VC path at: {}", path.display());
            }
            Err(MsvcEnvError::NoVisualStudio) => {
                println!("No Visual Studio installation found - this is expected if VS is not installed");
            }
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }

    #[test]
    #[serial]
    fn test_environment() {
        cleanup_cache();
        let msvc_env = MsvcEnv::new().unwrap();
        
        // This test will only pass if Visual Studio with VC tools is installed
        match msvc_env.environment(MsvcArch::X64) {
            Ok(env) => {
                // Print some important variables for debugging
                println!("Environment variables found: {}", env.vars.len());
            }
            Err(MsvcEnvError::NoVisualStudio) => {
                println!("No Visual Studio installation found - this is expected if VS is not installed");
            }
            Err(e) => panic!("Unexpected error: {}", e),
        }
    }
}
