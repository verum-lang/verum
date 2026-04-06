// Memory and CBGR Cross-Platform Tests

use super::detection::PlatformInfo;
use std::alloc::{alloc, dealloc, Layout};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_page_size() {
        let platform = PlatformInfo::detect();
        println!("Page size: {} bytes", platform.page_size);

        assert!(platform.page_size == 4096 || platform.page_size == 16384 || platform.page_size == 65536);
    }

    #[test]
    fn test_memory_allocation() {
        unsafe {
            let layout = Layout::from_size_align(1024, 8).unwrap();
            let ptr = alloc(layout);
            assert!(!ptr.is_null());

            // Write pattern
            for i in 0..1024 {
                ptr.add(i).write(i as u8);
            }

            // Verify
            for i in 0..1024 {
                assert_eq!(ptr.add(i).read(), i as u8);
            }

            dealloc(ptr, layout);
        }
    }

    #[test]
    fn test_large_allocation() {
        let size = 100 * 1024 * 1024; // 100MB
        let vec: Vec<u8> = vec![0; size];
        assert_eq!(vec.len(), size);
    }

    #[test]
    fn test_pointer_width() {
        let platform = PlatformInfo::detect();
        println!("Pointer width: {} bits", platform.pointer_width);

        assert!(platform.pointer_width == 32 || platform.pointer_width == 64);
        assert_eq!(std::mem::size_of::<usize>() * 8, platform.pointer_width);
    }

    #[test]
    fn test_endianness() {
        let platform = PlatformInfo::detect();
        println!("Endianness: {:?}", platform.endianness);

        let value: u32 = 0x12345678;
        let bytes = value.to_ne_bytes();

        #[cfg(target_endian = "little")]
        assert_eq!(bytes, [0x78, 0x56, 0x34, 0x12]);

        #[cfg(target_endian = "big")]
        assert_eq!(bytes, [0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn test_alignment() {
        #[repr(align(64))]
        struct Aligned {
            data: [u8; 64],
        }

        let aligned = Aligned { data: [0; 64] };
        let addr = &aligned as *const _ as usize;
        assert_eq!(addr % 64, 0);
    }

    #[test]
    #[cfg(unix)]
    fn test_mmap() {
        use std::ptr;

        unsafe {
            let size = 4096;
            let ptr = libc::mmap(
                ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            );

            assert_ne!(ptr, libc::MAP_FAILED);

            // Write data
            let data = ptr as *mut u8;
            data.write(42);

            assert_eq!(data.read(), 42);

            libc::munmap(ptr, size);
        }
    }

    #[test]
    fn test_stack_allocation() {
        const SIZE: usize = 1024;
        let _stack_array: [u8; SIZE] = [0; SIZE];
        // Stack allocated, no explicit free needed
    }

    #[test]
    fn test_memory_leaks_detection() {
        // Create and drop allocations
        for _ in 0..1000 {
            let _v: Vec<u8> = vec![0; 1024];
        }
        // Valgrind/ASAN would detect leaks here
    }
}
