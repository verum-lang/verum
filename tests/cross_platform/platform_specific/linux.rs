// Linux-Specific Tests

#[cfg(target_os = "linux")]
#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Read;

    #[test]
    fn test_proc_filesystem() {
        // Read /proc/self/status
        let mut file = File::open("/proc/self/status").unwrap();
        let mut content = String::new();
        file.read_to_string(&mut content).unwrap();

        assert!(content.contains("Name:"));
        println!("Process status:\n{}", &content[..200.min(content.len())]);
    }

    #[test]
    fn test_proc_cpuinfo() {
        let mut file = File::open("/proc/cpuinfo").unwrap();
        let mut content = String::new();
        file.read_to_string(&mut content).unwrap();

        assert!(content.contains("processor") || content.contains("cpu"));
    }

    #[test]
    fn test_proc_meminfo() {
        let mut file = File::open("/proc/meminfo").unwrap();
        let mut content = String::new();
        file.read_to_string(&mut content).unwrap();

        assert!(content.contains("MemTotal:"));
    }

    #[test]
    fn test_sysfs() {
        // Check sysfs availability
        let path = std::path::Path::new("/sys/class");
        assert!(path.exists());
    }

    #[test]
    fn test_epoll_available() {
        unsafe {
            let epfd = libc::epoll_create1(0);
            assert!(epfd >= 0);
            libc::close(epfd);
        }
    }

    #[test]
    fn test_eventfd() {
        unsafe {
            let efd = libc::eventfd(0, libc::EFD_NONBLOCK);
            assert!(efd >= 0);
            libc::close(efd);
        }
    }

    #[test]
    fn test_timerfd() {
        unsafe {
            let tfd = libc::timerfd_create(libc::CLOCK_MONOTONIC, 0);
            assert!(tfd >= 0);
            libc::close(tfd);
        }
    }

    #[test]
    fn test_inotify() {
        unsafe {
            let ifd = libc::inotify_init();
            assert!(ifd >= 0);
            libc::close(ifd);
        }
    }

    #[test]
    fn test_prctl() {
        unsafe {
            // Get thread name
            let mut name = [0u8; 16];
            let result = libc::prctl(libc::PR_GET_NAME, name.as_mut_ptr(), 0, 0, 0);
            assert_eq!(result, 0);
        }
    }

    #[test]
    fn test_capabilities() {
        // Check if we can read capabilities
        let path = std::path::Path::new("/proc/self/status");
        if let Ok(mut file) = File::open(path) {
            let mut content = String::new();
            file.read_to_string(&mut content).unwrap();
            if content.contains("CapEff:") {
                println!("Capabilities info available");
            }
        }
    }

    #[test]
    fn test_cgroups() {
        let cgroup_path = std::path::Path::new("/proc/self/cgroup");
        if cgroup_path.exists() {
            let mut file = File::open(cgroup_path).unwrap();
            let mut content = String::new();
            file.read_to_string(&mut content).unwrap();
            println!("Cgroup info available");
        }
    }

    #[test]
    fn test_namespaces() {
        let ns_path = std::path::Path::new("/proc/self/ns");
        if ns_path.exists() {
            println!("Namespaces supported");
        }
    }
}
