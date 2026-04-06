// verum_lld - C++ wrapper for LLD linker
//
// This file provides a C-compatible interface to LLD's linking functionality.
// It uses LLVM's CrashRecoveryContext for safe error handling.

#include "lld/Common/Driver.h"
#include "lld/Common/CommonLinkerContext.h"
#include "llvm/Support/CrashRecoveryContext.h"
#include "llvm/Support/raw_ostream.h"
#include "llvm/ADT/SmallString.h"

#include <cstring>
#include <vector>

// Register all LLD drivers we support
LLD_HAS_DRIVER(elf)

#ifdef LLD_HAS_MACHO
LLD_HAS_DRIVER(macho)
#endif

#ifdef LLD_HAS_COFF
LLD_HAS_DRIVER(coff)
#endif

#ifdef LLD_HAS_WASM
LLD_HAS_DRIVER(wasm)
#endif

// Output buffer for capturing stdout/stderr
struct OutputBuffer {
    char* data;
    size_t size;
    size_t capacity;
};

static void init_buffer(OutputBuffer* buf, size_t initial_capacity = 4096) {
    buf->data = (char*)malloc(initial_capacity);
    buf->size = 0;
    buf->capacity = initial_capacity;
    if (buf->data) {
        buf->data[0] = '\0';
    }
}

static void append_buffer(OutputBuffer* buf, const char* data, size_t len) {
    if (!buf->data) return;

    if (buf->size + len + 1 > buf->capacity) {
        size_t new_capacity = (buf->capacity * 2 > buf->size + len + 1)
            ? buf->capacity * 2
            : buf->size + len + 1 + 4096;
        char* new_data = (char*)realloc(buf->data, new_capacity);
        if (!new_data) return;
        buf->data = new_data;
        buf->capacity = new_capacity;
    }

    memcpy(buf->data + buf->size, data, len);
    buf->size += len;
    buf->data[buf->size] = '\0';
}

static void free_buffer(OutputBuffer* buf) {
    if (buf->data) {
        free(buf->data);
        buf->data = nullptr;
    }
    buf->size = 0;
    buf->capacity = 0;
}

// Custom raw_ostream that captures output to a buffer
class BufferOstream : public llvm::raw_ostream {
public:
    OutputBuffer* buffer;

    BufferOstream(OutputBuffer* buf) : buffer(buf) {
        SetUnbuffered();
    }

    void write_impl(const char* ptr, size_t size) override {
        append_buffer(buffer, ptr, size);
    }

    uint64_t current_pos() const override {
        return buffer ? buffer->size : 0;
    }
};

// Link result structure
struct LinkResult {
    bool success;
    char* stdout_data;
    size_t stdout_size;
    char* stderr_data;
    size_t stderr_size;
};

extern "C" {

// Initialize LLD (call once at startup)
void verum_lld_init() {
    // Nothing needed for now, but reserved for future initialization
}

// Free link result
void verum_lld_free_result(LinkResult* result) {
    if (result) {
        // Create temporary OutputBuffer structs to use free_buffer
        OutputBuffer stdout_buf = {result->stdout_data, result->stdout_size, result->stdout_size};
        OutputBuffer stderr_buf = {result->stderr_data, result->stderr_size, result->stderr_size};
        free_buffer(&stdout_buf);
        free_buffer(&stderr_buf);
        result->stdout_data = nullptr;
        result->stderr_data = nullptr;
        result->stdout_size = 0;
        result->stderr_size = 0;
    }
}

// Link ELF binary
//
// Arguments:
//   argv: Array of command-line arguments (like ld.lld)
//   argc: Number of arguments
//   result: Output structure for result and captured output
//
// Returns: true on success, false on failure
bool verum_lld_link_elf(const char** argv, size_t argc, LinkResult* result) {
    if (!result) return false;

    // Initialize buffers
    OutputBuffer stdout_buf, stderr_buf;
    init_buffer(&stdout_buf);
    init_buffer(&stderr_buf);

    BufferOstream stdout_stream(&stdout_buf);
    BufferOstream stderr_stream(&stderr_buf);

    bool success = false;
    bool canRunAgain = false;

    // Run LLD in crash recovery context
    {
        llvm::ArrayRef<const char*> args(argv, argc);
        llvm::CrashRecoveryContext crc;

        if (!crc.RunSafely([&]() {
            canRunAgain = lld::elf::link(args, stdout_stream, stderr_stream,
                                          /*exitEarly=*/false, /*disableOutput=*/false);
        })) {
            // Crash occurred
            append_buffer(&stderr_buf, "LLD crashed during linking\n", 28);
            success = false;
        } else {
            success = canRunAgain;
        }
    }

    // Cleanup linker context
    {
        llvm::CrashRecoveryContext crc;
        crc.RunSafely([&]() {
            lld::CommonLinkerContext::destroy();
        });
    }

    // Transfer ownership to result
    result->success = success;
    result->stdout_data = stdout_buf.data;
    result->stdout_size = stdout_buf.size;
    result->stderr_data = stderr_buf.data;
    result->stderr_size = stderr_buf.size;

    // Don't free buffers - ownership transferred to result
    stdout_buf.data = nullptr;
    stderr_buf.data = nullptr;

    return success;
}

#ifdef LLD_HAS_MACHO
// Link Mach-O binary (macOS)
bool verum_lld_link_macho(const char** argv, size_t argc, LinkResult* result) {
    if (!result) return false;

    OutputBuffer stdout_buf, stderr_buf;
    init_buffer(&stdout_buf);
    init_buffer(&stderr_buf);

    BufferOstream stdout_stream(&stdout_buf);
    BufferOstream stderr_stream(&stderr_buf);

    bool success = false;
    bool canRunAgain = false;

    {
        llvm::ArrayRef<const char*> args(argv, argc);
        llvm::CrashRecoveryContext crc;

        if (!crc.RunSafely([&]() {
            canRunAgain = lld::macho::link(args, stdout_stream, stderr_stream,
                                            false, false);
        })) {
            append_buffer(&stderr_buf, "LLD crashed during Mach-O linking\n", 35);
            success = false;
        } else {
            success = canRunAgain;
        }
    }

    {
        llvm::CrashRecoveryContext crc;
        crc.RunSafely([&]() {
            lld::CommonLinkerContext::destroy();
        });
    }

    result->success = success;
    result->stdout_data = stdout_buf.data;
    result->stdout_size = stdout_buf.size;
    result->stderr_data = stderr_buf.data;
    result->stderr_size = stderr_buf.size;

    stdout_buf.data = nullptr;
    stderr_buf.data = nullptr;

    return success;
}
#endif

#ifdef LLD_HAS_COFF
// Link COFF/PE binary (Windows)
bool verum_lld_link_coff(const char** argv, size_t argc, LinkResult* result) {
    if (!result) return false;

    OutputBuffer stdout_buf, stderr_buf;
    init_buffer(&stdout_buf);
    init_buffer(&stderr_buf);

    BufferOstream stdout_stream(&stdout_buf);
    BufferOstream stderr_stream(&stderr_buf);

    bool success = false;
    bool canRunAgain = false;

    {
        llvm::ArrayRef<const char*> args(argv, argc);
        llvm::CrashRecoveryContext crc;

        if (!crc.RunSafely([&]() {
            canRunAgain = lld::coff::link(args, stdout_stream, stderr_stream,
                                           false, false);
        })) {
            append_buffer(&stderr_buf, "LLD crashed during COFF linking\n", 33);
            success = false;
        } else {
            success = canRunAgain;
        }
    }

    {
        llvm::CrashRecoveryContext crc;
        crc.RunSafely([&]() {
            lld::CommonLinkerContext::destroy();
        });
    }

    result->success = success;
    result->stdout_data = stdout_buf.data;
    result->stdout_size = stdout_buf.size;
    result->stderr_data = stderr_buf.data;
    result->stderr_size = stderr_buf.size;

    stdout_buf.data = nullptr;
    stderr_buf.data = nullptr;

    return success;
}
#endif

#ifdef LLD_HAS_WASM
// Link WebAssembly module
bool verum_lld_link_wasm(const char** argv, size_t argc, LinkResult* result) {
    if (!result) return false;

    OutputBuffer stdout_buf, stderr_buf;
    init_buffer(&stdout_buf);
    init_buffer(&stderr_buf);

    BufferOstream stdout_stream(&stdout_buf);
    BufferOstream stderr_stream(&stderr_buf);

    bool success = false;
    bool canRunAgain = false;

    {
        llvm::ArrayRef<const char*> args(argv, argc);
        llvm::CrashRecoveryContext crc;

        if (!crc.RunSafely([&]() {
            canRunAgain = lld::wasm::link(args, stdout_stream, stderr_stream,
                                           false, false);
        })) {
            append_buffer(&stderr_buf, "LLD crashed during WASM linking\n", 33);
            success = false;
        } else {
            success = canRunAgain;
        }
    }

    {
        llvm::CrashRecoveryContext crc;
        crc.RunSafely([&]() {
            lld::CommonLinkerContext::destroy();
        });
    }

    result->success = success;
    result->stdout_data = stdout_buf.data;
    result->stdout_size = stdout_buf.size;
    result->stderr_data = stderr_buf.data;
    result->stderr_size = stderr_buf.size;

    stdout_buf.data = nullptr;
    stderr_buf.data = nullptr;

    return success;
}
#endif

// Get LLD version string
const char* verum_lld_version() {
    // Use LLVM version since LLD_VERSION_STRING is not always available
    return LLVM_VERSION_STRING;
}

} // extern "C"
