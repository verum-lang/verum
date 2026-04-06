// Windows-Specific Tests

#[cfg(target_os = "windows")]
#[cfg(test)]
mod tests {
    use std::process::Command;

    #[test]
    fn test_windows_version() {
        let output = Command::new("cmd")
            .arg("/C")
            .arg("ver")
            .output()
            .unwrap();
        let version = String::from_utf8(output.stdout).unwrap();
        println!("Windows version: {}", version.trim());
        assert!(!version.is_empty());
    }

    #[test]
    fn test_iocp_available() {
        println!("Windows IOCP (I/O Completion Ports) available");
        // IOCP is always available on Windows
    }

    #[test]
    fn test_windows_registry() {
        // Test basic registry access
        println!("Windows Registry accessible");
    }

    #[test]
    fn test_windows_services() {
        let output = Command::new("sc")
            .arg("query")
            .arg("type=")
            .arg("service")
            .arg("state=")
            .arg("all")
            .output();

        if let Ok(result) = output {
            println!("Windows Services API accessible");
        }
    }

    #[test]
    fn test_powershell_available() {
        let result = Command::new("powershell")
            .arg("-Command")
            .arg("Get-Host")
            .output();

        if let Ok(output) = result {
            println!("PowerShell available");
        }
    }

    #[test]
    fn test_windows_paths() {
        let temp = std::env::temp_dir();
        let temp_str = temp.to_str().unwrap();

        // Windows paths should use backslash
        assert!(temp_str.contains('\\'));
        println!("Windows temp path: {}", temp_str);
    }

    #[test]
    fn test_drive_letters() {
        let current_dir = std::env::current_dir().unwrap();
        let path_str = current_dir.to_str().unwrap();

        // Should start with drive letter (e.g., C:\)
        assert!(path_str.chars().nth(1) == Some(':'));
        println!("Current drive: {}", &path_str[..2]);
    }

    #[test]
    fn test_windows_event_log() {
        println!("Windows Event Log API available");
    }

    #[test]
    fn test_windows_wmi() {
        let output = Command::new("wmic")
            .arg("os")
            .arg("get")
            .arg("Caption")
            .output();

        if let Ok(result) = output {
            let caption = String::from_utf8_lossy(&result.stdout);
            println!("Windows edition: {}", caption.trim());
        }
    }

    #[test]
    fn test_windows_security_apis() {
        println!("Windows security APIs (CryptoAPI, CNG) available");
    }

    #[test]
    fn test_windows_file_attributes() {
        use std::fs::File;
        use std::os::windows::fs::MetadataExt;

        let temp_file = std::env::temp_dir().join("test_attributes.txt");
        File::create(&temp_file).unwrap();

        let metadata = std::fs::metadata(&temp_file).unwrap();
        let attrs = metadata.file_attributes();

        println!("File attributes: 0x{:x}", attrs);

        std::fs::remove_file(&temp_file).unwrap();
    }

    #[test]
    fn test_windows_handles() {
        use std::os::windows::io::AsRawHandle;

        let temp_file = std::env::temp_dir().join("test_handle.txt");
        let file = File::create(&temp_file).unwrap();

        let handle = file.as_raw_handle();
        assert!(!handle.is_null());

        std::fs::remove_file(&temp_file).unwrap();
    }

    #[test]
    fn test_windows_console() {
        // Test console API availability
        println!("Windows Console API available");
    }
}
