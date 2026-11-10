/* Process-wide Linux signal observation. This translation unit is freestanding
 * in both the hosted VM and a native image. A handler can interrupt any thread.
 * It touches only a permanently allocated, lock-free counter. */
typedef __INT64_TYPE__ Word;
typedef __UINT64_TYPE__ Bits;
typedef void (*Handler)(int);

struct Action {
    Handler handler;
    Bits flags;
    void (*restorer)(void);
    Bits mask;
};

enum { INACTIVE, CHANGING, ACTIVE };
struct Watch {
    unsigned state;
    Bits pending;
    struct Action previous;
};
static struct Watch watches[65];
_Static_assert(__atomic_always_lock_free(sizeof(Bits), 0), "handler requires lock-free counters");

#if defined(__x86_64__)
extern void verum_linux_restore_context(void);
__asm__(".text\n.hidden verum_linux_restore_context\n.type verum_linux_restore_context,@function\n"
        "verum_linux_restore_context:\n mov $15, %rax\n syscall\n");
static Word set_action(Word number, const struct Action *next, struct Action *previous) {
    register Word mask_size __asm__("r10") = sizeof(Bits);
    Word result;
    __asm__ volatile("syscall" : "=a"(result) : "0"(13L), "D"(number),
                     "S"(next), "d"(previous), "r"(mask_size) : "rcx", "r11", "memory");
    return result;
}
#elif defined(__aarch64__)
extern void verum_linux_restore_context(void);
__asm__(".text\n.hidden verum_linux_restore_context\n.type verum_linux_restore_context,%function\n"
        "verum_linux_restore_context:\n mov x8, #139\n svc #0\n");
static Word set_action(Word number, const struct Action *next, struct Action *previous) {
    register Word x0 __asm__("x0") = number;
    register const struct Action *x1 __asm__("x1") = next;
    register struct Action *x2 __asm__("x2") = previous;
    register Word x3 __asm__("x3") = sizeof(Bits);
    register Word x8 __asm__("x8") = 134;
    __asm__ volatile("svc #0" : "+r"(x0) : "r"(x1), "r"(x2), "r"(x3), "r"(x8) : "memory", "cc");
    return x0;
}
#else
#error "unsupported Linux rt_sigaction ABI"
#endif

static void observe(int number) {
    Bits *pending = &watches[number].pending;
    Bits count = __atomic_load_n(pending, __ATOMIC_RELAXED);
    while (count <= __INT64_MAX__ &&
           !__atomic_compare_exchange_n(pending, &count, count + 1, 1,
                                        __ATOMIC_RELAXED, __ATOMIC_RELAXED)) {}
}

/* The public affine handle serializes its own calls. The native reservation
 * also excludes independent handles which try to own the same disposition. */
Word verum_linux_signal_watch(Word number, Word operation) {
    if (number < 1 || number > 64) return -22; /* EINVAL */
    struct Watch *watch = &watches[number];
    if (operation == 1) {
        if (__atomic_load_n(&watch->state, __ATOMIC_ACQUIRE) != ACTIVE) return -9; /* EBADF */
        Bits count = __atomic_exchange_n(&watch->pending, 0, __ATOMIC_RELAXED);
        return count <= __INT64_MAX__ ? (Word)count : -75; /* EOVERFLOW */
    }
    if (operation != 0 && operation != 2) return -22;
    unsigned original = operation == 0 ? INACTIVE : ACTIVE;
    unsigned target = operation == 0 ? ACTIVE : INACTIVE;
    unsigned expected = original;
    if (!__atomic_compare_exchange_n(&watch->state, &expected, CHANGING, 0,
                                     __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE))
        return expected == INACTIVE ? -9 : -16; /* EBADF, EBUSY */
    Word result;
    if (operation == 0) {
        /* SA_RESTORER and SA_RESTART. The trampoline is part of this image. */
        const struct Action action = {observe, 0x04000000UL | 0x10000000UL, verum_linux_restore_context, 0};
        __atomic_store_n(&watch->pending, 0, __ATOMIC_RELAXED);
        result = set_action(number, &action, &watch->previous);
    } else {
        result = set_action(number, &watch->previous, 0);
    }
    __atomic_store_n(&watch->state, result == 0 ? target : original, __ATOMIC_RELEASE);
    return result;
}
