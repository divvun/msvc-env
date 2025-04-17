use std::path::{Path, PathBuf};
use std::process::Command;
use std::fs;
use thiserror::Error;

const VSWHERE_URL: &str = "https://github.com/microsoft/vswhere/releases/download/3.1.1/vswhere.exe";

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
}

pub struct MsvcEnv {
    vswhere_path: PathBuf,
}

impl MsvcEnv {
    pub fn new() -> Result<Self, MsvcEnvError> {
        // Create a cache directory in target
        let target_dir = PathBuf::from("target");
        let cache_dir = target_dir.join("msvc-env-cache");
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

    pub fn get_vc_path(&self) -> Result<PathBuf, MsvcEnvError> {
        let vs_path = self.find_visual_studio()?;
        let vc_path = vs_path.join("VC");
        
        if !vc_path.exists() {
            return Err(MsvcEnvError::NoVisualStudio);
        }

        Ok(vc_path)
    }
}

pub fn add(left: u64, right: u64) -> u64 {
    left + right
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
    fn test_get_vc_path() {
        cleanup_cache();
        let msvc_env = MsvcEnv::new().unwrap();
        
        // This test will only pass if Visual Studio with VC tools is installed
        match msvc_env.get_vc_path() {
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
}
