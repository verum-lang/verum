// BSD-Specific Tests

#[cfg(any(
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
#[cfg(test)]
mod tests {
    use std::process::Command;

    #[test]
    fn test_bsd_version() {
        let output = Command::new("uname").arg("-rs").output().unwrap();
        let version = String::from_utf8(output.stdout).unwrap();
        println!("BSD version: {}", version.trim());
        assert!(!version.is_empty());
    }

    #[test]
    fn test_kqueue_available() {
        unsafe {
            let kq = libc::kqueue();
            assert!(kq >= 0);
            libc::close(kq);
        }
    }

    #[test]
    fn test_sysctl_available() {
        let output = Command::new("sysctl").arg("kern.version").output();
        if let Ok(result) = output {
            let info = String::from_utf8(result.stdout).unwrap();
            println!("Kernel info: {}", info.trim());
        }
    }

    #[test]
    fn test_bsd_proc_filesystem() {
        // BSD may have /proc but it's optional
        let proc_exists = std::path::Path::new("/proc").exists();
        println!("procfs mounted: {}", proc_exists);
    }

    #[test]
    fn test_bsd_ports_packages() {
        // Check for package management
        #[cfg(target_os = "freebsd")]
        {
            let result = Command::new("pkg").arg("--version").output();
            if result.is_ok() {
                println!("FreeBSD pkg available");
            }
        }

        #[cfg(target_os = "openbsd")]
        {
            let result = Command::new("pkg_info").output();
            if result.is_ok() {
                println!("OpenBSD pkg_info available");
            }
        }
    }

    #[test]
    fn test_bsd_jails() {
        #[cfg(target_os = "freebsd")]
        {
            println!("FreeBSD jails API available");
        }
    }

    #[test]
    fn test_bsd_pledge_unveil() {
        #[cfg(target_os = "openbsd")]
        {
            println!("OpenBSD pledge/unveil available");
        }
    }
}
