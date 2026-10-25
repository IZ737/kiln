/* The Kiln runtime. The C backend prepends it to every generated C file;
 * the native backend compiles it once, into the runtime `kiln` carries.
 *
 * Values: Int is int64_t, Float is double, Bool is bool, Duration is int64_t
 * nanoseconds. Everything else is a pointer to memory owned by the Boehm
 * garbage collector. Generic code (type parameters, list elements, enum
 * fields, closure arguments) stores values in `kv`, a 64-bit slot. An
 * optional `T?` is a `kbox *` holding the value as a kv, or NULL for none.
 */
#include <gc.h>
#include <errno.h>
#include <inttypes.h>
#include <math.h>
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <dirent.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

/* Runtime functions and variables are `K_API`: `static` when the runtime is
 * prepended to generated C, and visible to the linker when the native
 * backend compiles it into an object of its own. */
#ifndef K_API
#define K_API static
#endif

typedef uint64_t kv;

static inline kv k_f2kv(double f) { kv v; memcpy(&v, &f, sizeof v); return v; }
static inline double k_kv2f(kv v) { double f; memcpy(&f, &v, sizeof f); return f; }
#define K_P2KV(p) ((kv)(uintptr_t)(p))
#define K_KV2P(t, v) ((t)(uintptr_t)(v))

K_API void *k_alloc(size_t n) {
    void *p = GC_MALLOC(n);
    if (!p) { fputs("panic: out of memory\n", stderr); exit(101); }
    return p;
}

K_API void *k_alloc_atomic(size_t n) {
    void *p = GC_MALLOC_ATOMIC(n);
    if (!p) { fputs("panic: out of memory\n", stderr); exit(101); }
    return p;
}

/* ----- strings ----- */

/* A string is a length and a pointer to the start of a buffer's bytes.
 * Strings never change, but a buffer can grow at its end: when `a + b`
 * finds that `a` ends where its buffer's bytes in use end and there is
 * room, `b` is copied after it and the result shares the buffer. Building
 * a string with `+=` in a loop is then linear rather than quadratic.
 *
 * Because of that, the byte after a string's last one is not always 0.
 * Pass `k_cstr(s)` to C functions that expect a NUL-terminated string. */
typedef struct kstr { int64_t len; char *data; } kstr;

/* A string and its buffer in one block; `data` points at `bytes`. */
typedef struct kstrbuf { kstr s; int64_t used, cap; char bytes[]; } kstrbuf;

K_API kstrbuf *k_strbuf_of(kstr *s) {
    return (kstrbuf *)(void *)(s->data - offsetof(kstrbuf, bytes));
}

/* A new string of `n` bytes with room for `cap`; the caller fills it. */
K_API kstr *k_str_alloc(int64_t n, int64_t cap) {
    /* The block points only into itself, so the collector need not scan it. */
    kstrbuf *b = k_alloc_atomic(sizeof(kstrbuf) + (size_t)cap + 1);
    b->s.len = n;
    b->s.data = b->bytes;
    b->used = n;
    b->cap = cap;
    b->bytes[n] = 0;
    return &b->s;
}

K_API kstr *k_str_n(const char *s, int64_t n) {
    kstr *r = k_str_alloc(n, n);
    memcpy(r->data, s, (size_t)n);
    return r;
}

K_API kstr *k_str(const char *s) { return k_str_n(s, (int64_t)strlen(s)); }

K_API const char *k_cstr(kstr *s) {
    if (s->data[s->len] == 0) return s->data;
    return k_str_n(s->data, s->len)->data;
}

K_API kstr *k_concat(kstr *a, kstr *b) {
    if (b->len == 0) return a;
    if (a->len == 0) return b;
    int64_t n = a->len + b->len;
    kstrbuf *buf = k_strbuf_of(a);
    if (buf->used == a->len && n <= buf->cap) {
        memcpy(buf->bytes + a->len, b->data, (size_t)b->len);
        buf->used = n;
        buf->bytes[n] = 0;
        kstr *r = k_alloc(sizeof(kstr));
        r->len = n;
        r->data = buf->bytes;
        return r;
    }
    /* Leave room to grow once a string is long enough to be being built. */
    kstr *r = k_str_alloc(n, n < 64 ? n : n * 2);
    memcpy(r->data, a->data, (size_t)a->len);
    memcpy(r->data + a->len, b->data, (size_t)b->len);
    return r;
}

K_API bool k_str_eq(kstr *a, kstr *b) {
    return a->len == b->len && memcmp(a->data, b->data, (size_t)a->len) == 0;
}

K_API int k_str_cmp(kstr *a, kstr *b) {
    int64_t n = a->len < b->len ? a->len : b->len;
    int c = memcmp(a->data, b->data, (size_t)n);
    if (c != 0) return c;
    return a->len < b->len ? -1 : a->len > b->len;
}

/* ----- panics and errors ----- */

K_API _Noreturn void k_panic(kstr *message) {
    fflush(stdout);
    fprintf(stderr, "panic: %s\n", k_cstr(message));
    exit(101);
}

K_API _Noreturn void k_panic_c(const char *message) { k_panic(k_str(message)); }

/* A failing call sets `k_failed` and stores the error; the caller checks the
 * flag at `try` or `catch`. `k_error_show` renders the error if it reaches
 * the top of `main`. */
K_API bool k_failed = false;
K_API kv k_error = 0;
K_API kstr *(*k_error_show)(kv) = NULL;

K_API void k_fail(kv error, kstr *(*show)(kv)) {
    k_failed = true;
    k_error = error;
    k_error_show = show;
}

K_API kv k_take_error(void) {
    k_failed = false;
    return k_error;
}

/* A standard library function that fails stores its message here and
 * returns; the generated code turns it into an `Error` and fails. */
K_API kstr *k_lib_error = NULL;

K_API void k_lib_fail(const char *what, kstr *path) {
    char buf[512];
    snprintf(buf, sizeof buf, "cannot %s %s: %s", what, k_cstr(path), strerror(errno));
    k_lib_error = k_str(buf);
}

/* ----- numbers ----- */

K_API int64_t k_add(int64_t a, int64_t b) {
    int64_t r;
    if (__builtin_add_overflow(a, b, &r)) k_panic_c("integer overflow");
    return r;
}
K_API int64_t k_sub(int64_t a, int64_t b) {
    int64_t r;
    if (__builtin_sub_overflow(a, b, &r)) k_panic_c("integer overflow");
    return r;
}
K_API int64_t k_mul(int64_t a, int64_t b) {
    int64_t r;
    if (__builtin_mul_overflow(a, b, &r)) k_panic_c("integer overflow");
    return r;
}
K_API int64_t k_div(int64_t a, int64_t b) {
    if (b == 0) k_panic_c("division by zero");
    if (a == INT64_MIN && b == -1) k_panic_c("integer overflow");
    return a / b;
}
K_API int64_t k_rem(int64_t a, int64_t b) {
    if (b == 0) k_panic_c("division by zero");
    if (b == -1) return 0;
    return a % b;
}
K_API int64_t k_neg(int64_t a) {
    if (a == INT64_MIN) k_panic_c("integer overflow");
    return -a;
}

K_API int64_t k_abs(int64_t a) { return a < 0 ? k_neg(a) : a; }
K_API int64_t k_min(int64_t a, int64_t b) { return a < b ? a : b; }
K_API int64_t k_max(int64_t a, int64_t b) { return a > b ? a : b; }

K_API int64_t k_pow(int64_t base, int64_t exp) {
    if (exp < 0) k_panic_c("a negative exponent");
    int64_t r = 1;
    while (exp > 0) {
        if (exp & 1) r = k_mul(r, base);
        exp >>= 1;
        if (exp > 0) base = k_mul(base, base);
    }
    return r;
}

K_API int64_t k_clamp(int64_t n, int64_t lo, int64_t hi) {
    if (lo > hi) k_panic_c("clamp: the low bound is above the high bound");
    return n < lo ? lo : n > hi ? hi : n;
}

K_API kstr *k_int_str(int64_t n) {
    char buf[32];
    int len = snprintf(buf, sizeof buf, "%" PRId64, n);
    return k_str_n(buf, len);
}

K_API kstr *k_float_str(double f) {
    char buf[64];
    if (f == floor(f) && fabs(f) < 1e15) {
        int len = snprintf(buf, sizeof buf, "%.1f", f);
        return k_str_n(buf, len);
    }
    int len = snprintf(buf, sizeof buf, "%.17g", f);
    /* Use the shortest form that reads back as the same number. */
    for (int p = 1; p <= 17; p++) {
        char tmp[64];
        int n = snprintf(tmp, sizeof tmp, "%.*g", p, f);
        if (strtod(tmp, NULL) == f) return k_str_n(tmp, n);
    }
    return k_str_n(buf, len);
}

K_API kstr *k_bool_str(bool b) { return k_str(b ? "true" : "false"); }

K_API kstr *k_float_fixed(double f, int64_t digits) {
    if (digits < 0 || digits > 20) k_panic_c("fixed: digits must be between 0 and 20");
    char buf[400];
    int len = snprintf(buf, sizeof buf, "%.*f", (int)digits, f);
    if (len < 0 || len >= (int)sizeof buf) return k_float_str(f);
    return k_str_n(buf, len);
}

K_API kstr *k_duration_str(int64_t ns) {
    static const struct { int64_t unit; const char *name; } units[] = {
        {3600000000000LL, "h"}, {60000000000LL, "m"}, {1000000000LL, "s"},
        {1000000LL, "ms"}, {1000LL, "us"}, {1LL, "ns"},
    };
    for (size_t i = 0; i < sizeof units / sizeof units[0]; i++) {
        if (ns != 0 && ns % units[i].unit == 0) {
            char buf[48];
            int len = snprintf(buf, sizeof buf, "%" PRId64 "%s", ns / units[i].unit, units[i].name);
            return k_str_n(buf, len);
        }
    }
    return k_str("0s");
}

K_API int64_t k_float_to_int(double f) {
    if (!(f >= -9.2233720368547758e18 && f < 9.2233720368547758e18)) k_panic_c("float is out of the Int range");
    return (int64_t)f;
}

/* ----- optionals: none is NULL, a value is boxed as a kv ----- */

typedef struct kbox { kv v; } kbox;

K_API kbox *k_box(kv v) {
    kbox *b = k_alloc(sizeof(kbox));
    b->v = v;
    return b;
}

/* ----- closures ----- */

typedef struct kclosure { kv (*fn)(void *env, kv *args); void *env; } kclosure;

K_API kclosure *k_closure(kv (*fn)(void *, kv *), void *env) {
    kclosure *c = k_alloc(sizeof(kclosure));
    c->fn = fn;
    c->env = env;
    return c;
}

static inline kv k_call1(kclosure *c, kv a) { kv args[1] = {a}; return c->fn(c->env, args); }

/* ----- enums ----- */

typedef struct kenum { int64_t tag; kv f[]; } kenum;

K_API kenum *k_enum(int64_t tag, int n, const kv *fields) {
    kenum *e = k_alloc(sizeof(kenum) + sizeof(kv) * (size_t)n);
    e->tag = tag;
    for (int i = 0; i < n; i++) e->f[i] = fields[i];
    return e;
}

/* ----- records: an array of slots, in the record type's field order ----- */

K_API kv *k_record(int n, const kv *fields) {
    kv *r = k_alloc(sizeof(kv) * (size_t)n);
    for (int i = 0; i < n; i++) r[i] = fields[i];
    return r;
}

/* ----- lists ----- */

typedef struct klist { int64_t len, cap; kv *items; } klist;

K_API klist *k_list(int64_t n, const kv *items) {
    klist *l = k_alloc(sizeof(klist));
    l->len = n;
    l->cap = n < 4 ? 4 : n;
    l->items = k_alloc(sizeof(kv) * (size_t)l->cap);
    for (int64_t i = 0; i < n; i++) l->items[i] = items[i];
    return l;
}

K_API void k_list_push(klist *l, kv item) {
    if (l->len == l->cap) {
        int64_t cap = l->cap * 2;
        kv *items = k_alloc(sizeof(kv) * (size_t)cap);
        memcpy(items, l->items, sizeof(kv) * (size_t)l->len);
        l->items = items;
        l->cap = cap;
    }
    l->items[l->len++] = item;
}

K_API kv k_list_at(klist *l, int64_t i) {
    if (i < 0 || i >= l->len) {
        char buf[96];
        snprintf(buf, sizeof buf, "index %" PRId64 " is out of range for a list of length %" PRId64, i, l->len);
        k_panic_c(buf);
    }
    return l->items[i];
}

K_API void k_list_set(klist *l, int64_t i, kv v) {
    k_list_at(l, i);
    l->items[i] = v;
}

K_API klist *k_range(int64_t lo, int64_t hi) {
    klist *l = k_list(0, NULL);
    for (int64_t i = lo; i < hi; i++) k_list_push(l, (kv)i);
    return l;
}

K_API kbox *k_list_get(klist *l, int64_t i) { return i >= 0 && i < l->len ? k_box(l->items[i]) : NULL; }
K_API kbox *k_list_first(klist *l) { return k_list_get(l, 0); }
K_API kbox *k_list_last(klist *l) { return k_list_get(l, l->len - 1); }

K_API klist *k_list_filter(klist *l, kclosure *keep) {
    klist *r = k_list(0, NULL);
    for (int64_t i = 0; i < l->len; i++)
        if ((bool)k_call1(keep, l->items[i])) k_list_push(r, l->items[i]);
    return r;
}

K_API klist *k_list_map(klist *l, kclosure *f) {
    klist *r = k_list(0, NULL);
    for (int64_t i = 0; i < l->len; i++) k_list_push(r, k_call1(f, l->items[i]));
    return r;
}

K_API kbox *k_list_find(klist *l, kclosure *f) {
    for (int64_t i = 0; i < l->len; i++)
        if ((bool)k_call1(f, l->items[i])) return k_box(l->items[i]);
    return NULL;
}

K_API bool k_list_any(klist *l, kclosure *f) {
    for (int64_t i = 0; i < l->len; i++)
        if ((bool)k_call1(f, l->items[i])) return true;
    return false;
}

K_API bool k_list_all(klist *l, kclosure *f) {
    for (int64_t i = 0; i < l->len; i++)
        if (!(bool)k_call1(f, l->items[i])) return false;
    return true;
}

K_API bool k_list_contains(klist *l, kv item, bool (*eq)(kv, kv)) {
    for (int64_t i = 0; i < l->len; i++)
        if (eq(l->items[i], item)) return true;
    return false;
}

K_API klist *k_list_reverse(klist *l) {
    klist *r = k_list(0, NULL);
    for (int64_t i = l->len - 1; i >= 0; i--) k_list_push(r, l->items[i]);
    return r;
}

/* A stable merge sort by precomputed keys. */
K_API klist *k_list_sort_by(klist *l, kclosure *key, int (*cmp)(kv, kv)) {
    int64_t n = l->len;
    kv *keys = k_alloc(sizeof(kv) * (size_t)(n ? n : 1));
    kv *vals = k_alloc(sizeof(kv) * (size_t)(n ? n : 1));
    kv *tk = k_alloc(sizeof(kv) * (size_t)(n ? n : 1));
    kv *tv = k_alloc(sizeof(kv) * (size_t)(n ? n : 1));
    for (int64_t i = 0; i < n; i++) { vals[i] = l->items[i]; keys[i] = k_call1(key, vals[i]); }
    for (int64_t w = 1; w < n; w *= 2) {
        for (int64_t lo = 0; lo < n; lo += 2 * w) {
            int64_t mid = lo + w < n ? lo + w : n, hi = lo + 2 * w < n ? lo + 2 * w : n;
            int64_t i = lo, j = mid, k = lo;
            while (i < mid && j < hi) {
                if (cmp(keys[j], keys[i]) < 0) { tk[k] = keys[j]; tv[k++] = vals[j++]; }
                else { tk[k] = keys[i]; tv[k++] = vals[i++]; }
            }
            while (i < mid) { tk[k] = keys[i]; tv[k++] = vals[i++]; }
            while (j < hi) { tk[k] = keys[j]; tv[k++] = vals[j++]; }
        }
        kv *t = keys; keys = tk; tk = t;
        t = vals; vals = tv; tv = t;
    }
    return k_list(n, vals);
}

K_API kstr *k_list_join(klist *l, kstr *sep, kstr *(*show)(kv)) {
    kstr *r = k_str("");
    for (int64_t i = 0; i < l->len; i++) {
        if (i > 0) r = k_concat(r, sep);
        r = k_concat(r, show(l->items[i]));
    }
    return r;
}

/* `[a, b]`, with each item shown by `show`. */
K_API kstr *k_list_show(klist *l, kstr *(*show)(kv)) {
    kstr *r = k_concat(k_str("["), k_list_join(l, k_str(", "), show));
    return k_concat(r, k_str("]"));
}

static inline kv k_call2(kclosure *c, kv a, kv b) { kv args[2] = {a, b}; return c->fn(c->env, args); }

K_API kbox *k_list_pop(klist *l) { return l->len > 0 ? k_box(l->items[--l->len]) : NULL; }

K_API void k_list_insert(klist *l, int64_t i, kv item) {
    if (i < 0 || i > l->len) {
        char buf[96];
        snprintf(buf, sizeof buf, "index %" PRId64 " is out of range for inserting into a list of length %" PRId64, i, l->len);
        k_panic_c(buf);
    }
    k_list_push(l, item);
    memmove(l->items + i + 1, l->items + i, sizeof(kv) * (size_t)(l->len - 1 - i));
    l->items[i] = item;
}

K_API kv k_list_remove_at(klist *l, int64_t i) {
    kv v = k_list_at(l, i);
    memmove(l->items + i, l->items + i + 1, sizeof(kv) * (size_t)(l->len - 1 - i));
    l->len--;
    return v;
}

K_API kbox *k_list_index_of(klist *l, kv item, bool (*eq)(kv, kv)) {
    for (int64_t i = 0; i < l->len; i++)
        if (eq(l->items[i], item)) return k_box((kv)i);
    return NULL;
}

/* Items from `start` up to `end`, both clamped to the list. */
K_API klist *k_list_slice(klist *l, int64_t start, int64_t end) {
    if (start < 0) start = 0;
    if (end > l->len) end = l->len;
    if (end < start) end = start;
    return k_list(end - start, l->items + start);
}

K_API klist *k_list_take(klist *l, int64_t n) { return k_list_slice(l, 0, n); }
K_API klist *k_list_skip(klist *l, int64_t n) { return k_list_slice(l, n, l->len); }

K_API klist *k_list_concat(klist *a, klist *b) {
    klist *r = k_list(a->len, a->items);
    for (int64_t i = 0; i < b->len; i++) k_list_push(r, b->items[i]);
    return r;
}

K_API kv k_identity(void *env, kv *args) { (void)env; return args[0]; }

K_API klist *k_list_sort(klist *l, int (*cmp)(kv, kv)) {
    kclosure id = {k_identity, NULL};
    return k_list_sort_by(l, &id, cmp);
}

/* The smallest item, or with `sign` -1 the largest; the first one on a tie. */
K_API kbox *k_list_extreme(klist *l, int (*cmp)(kv, kv), int sign) {
    if (l->len == 0) return NULL;
    kv best = l->items[0];
    for (int64_t i = 1; i < l->len; i++)
        if (sign * cmp(l->items[i], best) < 0) best = l->items[i];
    return k_box(best);
}

K_API int64_t k_list_count(klist *l, kclosure *f) {
    int64_t n = 0;
    for (int64_t i = 0; i < l->len; i++)
        if ((bool)k_call1(f, l->items[i])) n++;
    return n;
}

K_API kv k_list_fold(klist *l, kv acc, kclosure *f) {
    for (int64_t i = 0; i < l->len; i++) acc = k_call2(f, acc, l->items[i]);
    return acc;
}

K_API klist *k_list_flat_map(klist *l, kclosure *f) {
    klist *r = k_list(0, NULL);
    for (int64_t i = 0; i < l->len; i++) {
        klist *part = K_KV2P(klist *, k_call1(f, l->items[i]));
        for (int64_t j = 0; j < part->len; j++) k_list_push(r, part->items[j]);
    }
    return r;
}

/* ----- maps and sets -----
 *
 * A hash map from kv to kv that keeps keys in insertion order. Entries live
 * in arrays in the order they were added; `index` is an open-addressing
 * table of entry positions (-1 empty, -2 deleted). A set is a map whose
 * values are unused. */

typedef uint64_t (*k_hash_fn)(kv);
typedef bool (*k_eq_fn)(kv, kv);

typedef struct kmap {
    int64_t len, used, cap;
    kv *keys, *vals;
    bool *live;
    int64_t *index, slots;
    k_hash_fn hash;
    k_eq_fn eq;
} kmap;

K_API uint64_t k_hash_int(kv v) {
    uint64_t x = v;
    x ^= x >> 33; x *= 0xff51afd7ed558ccdULL;
    x ^= x >> 33; x *= 0xc4ceb9fe1a85ec53ULL;
    return x ^ (x >> 33);
}

/* Mixes the hash of one more part into `h`; the order of parts matters. */
K_API uint64_t k_hash_combine(uint64_t h, uint64_t x) {
    return k_hash_int(h ^ (x + 0x9e3779b97f4a7c15ULL + (h << 6) + (h >> 2)));
}

K_API uint64_t k_hash_float(kv v) {
    double f = k_kv2f(v);
    if (f == 0) f = 0; /* -0.0 and 0.0 are equal, so they hash the same */
    return k_hash_int(k_f2kv(f));
}

K_API uint64_t k_hash_str(kv v) {
    kstr *s = K_KV2P(kstr *, v);
    uint64_t h = 1469598103934665603ULL;
    for (int64_t i = 0; i < s->len; i++) { h ^= (unsigned char)s->data[i]; h *= 1099511628211ULL; }
    return h;
}

K_API kmap *k_map_new(k_hash_fn hash, k_eq_fn eq) {
    kmap *m = k_alloc(sizeof(kmap));
    m->cap = 8;
    m->keys = k_alloc(sizeof(kv) * 8);
    m->vals = k_alloc(sizeof(kv) * 8);
    m->live = k_alloc_atomic(8);
    m->slots = 16;
    m->index = k_alloc_atomic(sizeof(int64_t) * 16);
    for (int i = 0; i < 16; i++) m->index[i] = -1;
    m->hash = hash;
    m->eq = eq;
    return m;
}

/* The index slot holding `key`, or the empty slot where it would go. */
K_API int64_t k_map_slot(kmap *m, kv key) {
    uint64_t mask = (uint64_t)m->slots - 1;
    uint64_t i = m->hash(key) & mask;
    int64_t tomb = -1;
    for (;;) {
        int64_t e = m->index[i];
        if (e == -1) return tomb >= 0 ? tomb : (int64_t)i;
        if (e == -2) { if (tomb < 0) tomb = (int64_t)i; }
        else if (m->eq(m->keys[e], key)) return (int64_t)i;
        i = (i + 1) & mask;
    }
}

/* Rebuilds the entry arrays without deleted entries, and the index with
 * room for `cap` entries. */
K_API void k_map_rebuild(kmap *m, int64_t cap) {
    kv *keys = k_alloc(sizeof(kv) * (size_t)cap);
    kv *vals = k_alloc(sizeof(kv) * (size_t)cap);
    bool *live = k_alloc_atomic((size_t)cap);
    int64_t n = 0;
    for (int64_t i = 0; i < m->used; i++) {
        if (!m->live[i]) continue;
        keys[n] = m->keys[i]; vals[n] = m->vals[i]; live[n] = true; n++;
    }
    m->keys = keys; m->vals = vals; m->live = live;
    m->used = n; m->cap = cap;
    m->slots = 16;
    while (m->slots < cap * 2) m->slots *= 2;
    m->index = k_alloc_atomic(sizeof(int64_t) * (size_t)m->slots);
    for (int64_t i = 0; i < m->slots; i++) m->index[i] = -1;
    for (int64_t i = 0; i < n; i++) m->index[k_map_slot(m, keys[i])] = i;
}

K_API void k_map_set(kmap *m, kv key, kv val) {
    int64_t s = k_map_slot(m, key);
    int64_t e = m->index[s];
    if (e >= 0) { m->vals[e] = val; return; }
    if (m->used == m->cap) {
        k_map_rebuild(m, m->len * 2 > 8 ? m->len * 2 : 8);
        s = k_map_slot(m, key);
    }
    e = m->used++;
    m->keys[e] = key; m->vals[e] = val; m->live[e] = true;
    m->index[s] = e;
    m->len++;
}

K_API kbox *k_map_get(kmap *m, kv key) {
    int64_t e = m->index[k_map_slot(m, key)];
    return e >= 0 ? k_box(m->vals[e]) : NULL;
}

K_API kv k_map_at(kmap *m, kv key) {
    kbox *b = k_map_get(m, key);
    if (!b) k_panic_c("the key is not in the map");
    return b->v;
}

K_API bool k_map_contains(kmap *m, kv key) { return m->index[k_map_slot(m, key)] >= 0; }

K_API kbox *k_map_remove(kmap *m, kv key) {
    int64_t s = k_map_slot(m, key);
    int64_t e = m->index[s];
    if (e < 0) return NULL;
    kbox *old = k_box(m->vals[e]);
    m->index[s] = -2;
    m->live[e] = false;
    m->len--;
    return old;
}

K_API klist *k_map_keys(kmap *m) {
    klist *l = k_list(0, NULL);
    for (int64_t i = 0; i < m->used; i++) if (m->live[i]) k_list_push(l, m->keys[i]);
    return l;
}

K_API klist *k_map_values(kmap *m) {
    klist *l = k_list(0, NULL);
    for (int64_t i = 0; i < m->used; i++) if (m->live[i]) k_list_push(l, m->vals[i]);
    return l;
}

/* `{ key, value }` records, in insertion order. */
K_API klist *k_map_entries(kmap *m) {
    klist *l = k_list(0, NULL);
    for (int64_t i = 0; i < m->used; i++) {
        if (!m->live[i]) continue;
        kv *e = k_alloc(sizeof(kv) * 2);
        e[0] = m->keys[i];
        e[1] = m->vals[i];
        k_list_push(l, K_P2KV(e));
    }
    return l;
}

K_API void k_set_add(kmap *m, kv item) { k_map_set(m, item, 0); }
K_API bool k_set_remove(kmap *m, kv item) { return k_map_remove(m, item) != NULL; }

/* `{"a": 1, "b": 2}`, or `{1, 2}` for a set when `show_val` is NULL. */
K_API kstr *k_map_show(kmap *m, kstr *(*show_key)(kv), kstr *(*show_val)(kv)) {
    kstr *r = k_str("{");
    bool first = true;
    for (int64_t i = 0; i < m->used; i++) {
        if (!m->live[i]) continue;
        if (!first) r = k_concat(r, k_str(", "));
        first = false;
        r = k_concat(r, show_key(m->keys[i]));
        if (show_val) { r = k_concat(r, k_str(": ")); r = k_concat(r, show_val(m->vals[i])); }
    }
    return k_concat(r, k_str("}"));
}

/* Maps are equal when they hold the same keys with equal values, in any
 * order; sets when they hold the same items. */
K_API bool k_map_eq(kmap *a, kmap *b, k_eq_fn val_eq) {
    if (a->len != b->len) return false;
    for (int64_t i = 0; i < a->used; i++) {
        if (!a->live[i]) continue;
        int64_t e = b->index[k_map_slot(b, a->keys[i])];
        if (e < 0) return false;
        if (val_eq && !val_eq(a->vals[i], b->vals[e])) return false;
    }
    return true;
}

/* ----- string methods ----- */

K_API int64_t k_str_len(kstr *s) {
    int64_t n = 0;
    for (int64_t i = 0; i < s->len; i++)
        if ((s->data[i] & 0xC0) != 0x80) n++;
    return n;
}

K_API kstr *k_str_upper(kstr *s) {
    kstr *r = k_str_n(s->data, s->len);
    for (int64_t i = 0; i < r->len; i++)
        if (r->data[i] >= 'a' && r->data[i] <= 'z') r->data[i] -= 32;
    return r;
}

K_API kstr *k_str_lower(kstr *s) {
    kstr *r = k_str_n(s->data, s->len);
    for (int64_t i = 0; i < r->len; i++)
        if (r->data[i] >= 'A' && r->data[i] <= 'Z') r->data[i] += 32;
    return r;
}

K_API bool k_is_space(char c) { return c == ' ' || c == '\t' || c == '\n' || c == '\r'; }

K_API kstr *k_str_trim(kstr *s) {
    int64_t a = 0, b = s->len;
    while (a < b && k_is_space(s->data[a])) a++;
    while (b > a && k_is_space(s->data[b - 1])) b--;
    return k_str_n(s->data + a, b - a);
}

K_API int64_t k_str_find(kstr *s, kstr *part, int64_t from) {
    if (part->len == 0) return from;
    for (int64_t i = from; i + part->len <= s->len; i++)
        if (memcmp(s->data + i, part->data, (size_t)part->len) == 0) return i;
    return -1;
}

K_API bool k_str_contains(kstr *s, kstr *part) { return k_str_find(s, part, 0) >= 0; }

K_API bool k_str_starts_with(kstr *s, kstr *p) {
    return p->len <= s->len && memcmp(s->data, p->data, (size_t)p->len) == 0;
}

K_API bool k_str_ends_with(kstr *s, kstr *p) {
    return p->len <= s->len && memcmp(s->data + s->len - p->len, p->data, (size_t)p->len) == 0;
}

K_API kstr *k_str_replace(kstr *s, kstr *from, kstr *to) {
    if (from->len == 0) return s;
    kstr *r = k_str("");
    int64_t i = 0, j;
    while ((j = k_str_find(s, from, i)) >= 0) {
        r = k_concat(r, k_str_n(s->data + i, j - i));
        r = k_concat(r, to);
        i = j + from->len;
    }
    return k_concat(r, k_str_n(s->data + i, s->len - i));
}

K_API klist *k_str_split(kstr *s, kstr *sep) {
    klist *r = k_list(0, NULL);
    if (sep->len == 0) {
        for (int64_t i = 0; i < s->len; i++) k_list_push(r, K_P2KV(k_str_n(s->data + i, 1)));
        return r;
    }
    int64_t i = 0, j;
    while ((j = k_str_find(s, sep, i)) >= 0) {
        k_list_push(r, K_P2KV(k_str_n(s->data + i, j - i)));
        i = j + sep->len;
    }
    k_list_push(r, K_P2KV(k_str_n(s->data + i, s->len - i)));
    return r;
}

K_API kbox *k_str_to_int(kstr *s) {
    if (s->len == 0) return NULL;
    char *end;
    errno = 0;
    const char *d = k_cstr(s);
    long long n = strtoll(d, &end, 10);
    if (errno != 0 || end != d + s->len) return NULL;
    return k_box((kv)(int64_t)n);
}

K_API kbox *k_str_to_float(kstr *s) {
    if (s->len == 0 || k_is_space(s->data[0])) return NULL;
    char *end;
    errno = 0;
    const char *d = k_cstr(s);
    double f = strtod(d, &end);
    if (errno == ERANGE || end != d + s->len || isnan(f) || isinf(f)) return NULL;
    return k_box(k_f2kv(f));
}

/* The byte offset of character `i`, clamped to the string. */
K_API int64_t k_str_offset(kstr *s, int64_t i) {
    if (i <= 0) return 0;
    int64_t n = 0;
    for (int64_t b = 0; b < s->len; b++)
        if ((s->data[b] & 0xC0) != 0x80 && n++ == i) return b;
    return s->len;
}

/* The number of characters in the first `b` bytes. */
K_API int64_t k_str_chars_before(kstr *s, int64_t b) {
    int64_t n = 0;
    for (int64_t i = 0; i < b; i++)
        if ((s->data[i] & 0xC0) != 0x80) n++;
    return n;
}

K_API kbox *k_str_index_of(kstr *s, kstr *part) {
    int64_t b = k_str_find(s, part, 0);
    return b < 0 ? NULL : k_box((kv)k_str_chars_before(s, b));
}

/* Characters from `start` up to `end`, both clamped to the string. */
K_API kstr *k_str_slice(kstr *s, int64_t start, int64_t end) {
    int64_t a = k_str_offset(s, start), b = k_str_offset(s, end);
    return b > a ? k_str_n(s->data + a, b - a) : k_str("");
}

K_API klist *k_str_chars(kstr *s) {
    klist *r = k_list(0, NULL);
    for (int64_t i = 0; i < s->len;) {
        int64_t j = i + 1;
        while (j < s->len && (s->data[j] & 0xC0) == 0x80) j++;
        k_list_push(r, K_P2KV(k_str_n(s->data + i, j - i)));
        i = j;
    }
    return r;
}

/* Lines split on "\n" or "\r\n"; a final line break does not start an empty line. */
K_API klist *k_str_lines(kstr *s) {
    klist *r = k_list(0, NULL);
    int64_t i = 0;
    while (i < s->len) {
        int64_t j = i;
        while (j < s->len && s->data[j] != '\n') j++;
        int64_t e = j > i && s->data[j - 1] == '\r' ? j - 1 : j;
        k_list_push(r, K_P2KV(k_str_n(s->data + i, e - i)));
        i = j + 1;
    }
    return r;
}

K_API kstr *k_str_repeat(kstr *s, int64_t n) {
    if (n < 0) k_panic_c("repeat: the count is negative");
    int64_t total = k_mul(s->len, n);
    kstr *r = k_str_alloc(total, total);
    for (int64_t i = 0; i < n; i++) memcpy(r->data + i * s->len, s->data, (size_t)s->len);
    return r;
}

/* `fill` repeated before (or after) `s` until it is `width` characters long. */
K_API kstr *k_str_pad(kstr *s, int64_t width, kstr *fill, bool start) {
    int64_t have = k_str_len(s), step = k_str_len(fill);
    if (have >= width || step == 0) return s;
    kstr *pad = k_str_slice(k_str_repeat(fill, (width - have + step - 1) / step), 0, width - have);
    return start ? k_concat(pad, s) : k_concat(s, pad);
}

K_API kstr *k_str_pad_start(kstr *s, int64_t w, kstr *fill) { return k_str_pad(s, w, fill, true); }
K_API kstr *k_str_pad_end(kstr *s, int64_t w, kstr *fill) { return k_str_pad(s, w, fill, false); }

K_API kstr *k_str_trim_start(kstr *s) {
    int64_t a = 0;
    while (a < s->len && k_is_space(s->data[a])) a++;
    return k_str_n(s->data + a, s->len - a);
}

K_API kstr *k_str_trim_end(kstr *s) {
    int64_t b = s->len;
    while (b > 0 && k_is_space(s->data[b - 1])) b--;
    return k_str_n(s->data, b);
}

/* `"text"` with quotes and escapes, for strings inside lists and structs. */
K_API kstr *k_str_quote(kstr *s) {
    kstr *r = k_str("\"");
    for (int64_t i = 0; i < s->len; i++) {
        char c = s->data[i];
        const char *esc = c == '"' ? "\\\"" : c == '\\' ? "\\\\" : c == '\n' ? "\\n" : c == '\t' ? "\\t" : NULL;
        r = esc ? k_concat(r, k_str(esc)) : k_concat(r, k_str_n(&c, 1));
    }
    return k_concat(r, k_str("\""));
}

/* ----- built-in functions ----- */

K_API void k_print(kstr *s) {
    fwrite(s->data, 1, (size_t)s->len, stdout);
    fputc('\n', stdout);
}

K_API kstr *k_env(kstr *name) {
    const char *v = getenv(k_cstr(name));
    return k_str(v ? v : "");
}

K_API void k_log(const char *level, kstr *message, int n, kstr **data) {
    fflush(stdout);
    fprintf(stderr, "%s %s", level, k_cstr(message));
    for (int i = 0; i < n; i++) fprintf(stderr, " %s", k_cstr(data[i]));
    fputc('\n', stderr);
}

/* ----- time: instants are int64_t nanoseconds since 1970, in UTC ----- */

#define K_NS_PER_SEC INT64_C(1000000000)
#define K_NS_PER_DAY (INT64_C(86400) * K_NS_PER_SEC)

K_API int64_t k_floor_div(int64_t a, int64_t b) { return a / b - (a % b != 0 && (a < 0) != (b < 0)); }

K_API int64_t k_time_now(void) {
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (int64_t)ts.tv_sec * K_NS_PER_SEC + ts.tv_nsec;
}

K_API void k_time_sleep(int64_t ns) {
    if (ns <= 0) return;
    struct timespec ts = {(time_t)(ns / K_NS_PER_SEC), (long)(ns % K_NS_PER_SEC)};
    while (nanosleep(&ts, &ts) != 0 && errno == EINTR) {}
}

/* Days since 1970-01-01 to a civil date and back (Howard Hinnant's
 * algorithms, valid for every day an int64_t can hold here). */
K_API void k_civil(int64_t days, int64_t *y, int64_t *m, int64_t *d) {
    days += 719468;
    int64_t era = k_floor_div(days, 146097);
    int64_t doe = days - era * 146097;
    int64_t yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    int64_t doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    int64_t mp = (5 * doy + 2) / 153;
    *d = doy - (153 * mp + 2) / 5 + 1;
    *m = mp < 10 ? mp + 3 : mp - 9;
    *y = yoe + era * 400 + (*m <= 2);
}

K_API int64_t k_days_from_civil(int64_t y, int64_t m, int64_t d) {
    y -= m <= 2;
    int64_t era = k_floor_div(y, 400);
    int64_t yoe = y - era * 400;
    int64_t doy = (153 * (m > 2 ? m - 3 : m + 9) + 2) / 5 + d - 1;
    int64_t doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + doe - 719468;
}

/* The parts of an instant: year, month, day, hour, minute, second,
 * nanosecond and weekday (0 is Sunday). */
typedef struct ktm { int64_t year, month, day, hour, minute, second, nano, weekday; } ktm;

K_API ktm k_time_parts(int64_t ns) {
    ktm t;
    int64_t days = k_floor_div(ns, K_NS_PER_DAY);
    int64_t rest = ns - days * K_NS_PER_DAY;
    k_civil(days, &t.year, &t.month, &t.day);
    t.hour = rest / (3600 * K_NS_PER_SEC);
    t.minute = rest / (60 * K_NS_PER_SEC) % 60;
    t.second = rest / K_NS_PER_SEC % 60;
    t.nano = rest % K_NS_PER_SEC;
    t.weekday = ((days % 7) + 11) % 7;
    return t;
}

K_API int64_t k_time_year(int64_t ns) { return k_time_parts(ns).year; }
K_API int64_t k_time_month(int64_t ns) { return k_time_parts(ns).month; }
K_API int64_t k_time_day(int64_t ns) { return k_time_parts(ns).day; }
K_API int64_t k_time_unix(int64_t ns) { return k_floor_div(ns, K_NS_PER_SEC); }

K_API kstr *k_time_iso(int64_t ns) {
    ktm t = k_time_parts(ns);
    char buf[64];
    int len = snprintf(buf, sizeof buf, "%04" PRId64 "-%02" PRId64 "-%02" PRId64 "T%02" PRId64 ":%02" PRId64 ":%02" PRId64,
                       t.year, t.month, t.day, t.hour, t.minute, t.second);
    if (t.nano != 0) {
        int digits = 9;
        int64_t frac = t.nano;
        while (frac % 10 == 0) { frac /= 10; digits--; }
        len += snprintf(buf + len, sizeof buf - (size_t)len, ".%0*" PRId64, digits, frac);
    }
    buf[len++] = 'Z';
    return k_str_n(buf, len);
}

K_API const char *k_month_names[] = {"January", "February", "March", "April", "May", "June", "July",
                                      "August", "September", "October", "November", "December"};
K_API const char *k_day_names[] = {"Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"};

/* Formats with a date pattern such as "MMM d, yyyy". A run of one letter
 * is one field; text in single quotes is copied, and '' is a quote. */
K_API kstr *k_time_format(int64_t ns, kstr *pattern) {
    ktm t = k_time_parts(ns);
    size_t cap = 64, len = 0;
    char *out = k_alloc_atomic(cap);
#define K_PUT(s, n) do { size_t n_ = (n); while (len + n_ + 1 > cap) { char *o = k_alloc_atomic(cap * 2); memcpy(o, out, len); out = o; cap *= 2; } memcpy(out + len, (s), n_); len += n_; } while (0)
    const char *p = pattern->data;
    const char *end = p + pattern->len;
    while (p < end) {
        char c = *p;
        if (c == '\'') {
            p++;
            if (p < end && *p == '\'') { K_PUT("'", 1); p++; continue; }
            while (p < end) {
                if (*p == '\'') {
                    if (p + 1 < end && p[1] == '\'') { K_PUT("'", 1); p += 2; continue; }
                    p++;
                    break;
                }
                K_PUT(p, 1);
                p++;
            }
            continue;
        }
        if (!((c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z'))) { K_PUT(p, 1); p++; continue; }
        int n = 0;
        while (p < end && *p == c) { n++; p++; }
        char buf[32];
        int w = -1;
        switch (c) {
        case 'y':
            if (n == 2) w = snprintf(buf, sizeof buf, "%02" PRId64, ((t.year % 100) + 100) % 100);
            else w = snprintf(buf, sizeof buf, "%0*" PRId64, n, t.year);
            break;
        case 'M':
            if (n >= 4) { K_PUT(k_month_names[t.month - 1], strlen(k_month_names[t.month - 1])); continue; }
            if (n == 3) { K_PUT(k_month_names[t.month - 1], 3); continue; }
            w = snprintf(buf, sizeof buf, "%0*" PRId64, n, t.month);
            break;
        case 'd': w = snprintf(buf, sizeof buf, "%0*" PRId64, n, t.day); break;
        case 'E':
            if (n >= 4) { K_PUT(k_day_names[t.weekday], strlen(k_day_names[t.weekday])); continue; }
            K_PUT(k_day_names[t.weekday], 3);
            continue;
        case 'H': w = snprintf(buf, sizeof buf, "%0*" PRId64, n, t.hour); break;
        case 'h': w = snprintf(buf, sizeof buf, "%0*" PRId64, n, (t.hour + 11) % 12 + 1); break;
        case 'a': K_PUT(t.hour < 12 ? "AM" : "PM", 2); continue;
        case 'm': w = snprintf(buf, sizeof buf, "%0*" PRId64, n, t.minute); break;
        case 's': w = snprintf(buf, sizeof buf, "%0*" PRId64, n, t.second); break;
        case 'S': {
            /* Fractions of a second, to n digits. */
            int64_t frac = t.nano;
            for (int i = n; i < 9; i++) frac /= 10;
            w = snprintf(buf, sizeof buf, "%0*" PRId64, n > 9 ? 9 : n, frac);
            break;
        }
        default:
            /* Letters that are not fields are copied. */
            for (int i = 0; i < n; i++) K_PUT(&c, 1);
            continue;
        }
        K_PUT(buf, (size_t)w);
    }
#undef K_PUT
    out[len] = 0;
    return k_str_n(out, (int64_t)len);
}

K_API bool k_digits(const char **p, const char *end, int n, int64_t *out) {
    int64_t v = 0;
    for (int i = 0; i < n; i++) {
        if (*p >= end || **p < '0' || **p > '9') return false;
        v = v * 10 + (**p - '0');
        (*p)++;
    }
    *out = v;
    return true;
}

/* Reads an RFC 3339 time, or a date alone as midnight UTC. */
K_API int64_t k_time_parse(kstr *text) {
    const char *p = text->data, *end = p + text->len;
    int64_t y, mo, d, h = 0, mi = 0, s = 0, nano = 0, offset = 0;
    bool ok = k_digits(&p, end, 4, &y) && p < end && *p++ == '-' && k_digits(&p, end, 2, &mo) && p < end &&
              *p++ == '-' && k_digits(&p, end, 2, &d);
    if (ok && p < end) {
        ok = (*p == 'T' || *p == 't' || *p == ' ') && (p++, k_digits(&p, end, 2, &h)) && p < end && *p++ == ':' &&
             k_digits(&p, end, 2, &mi) && p < end && *p++ == ':' && k_digits(&p, end, 2, &s);
        if (ok && p < end && *p == '.') {
            p++;
            int n = 0;
            while (p < end && *p >= '0' && *p <= '9') {
                if (n < 9) nano = nano * 10 + (*p - '0');
                n++;
                p++;
            }
            ok = n > 0;
            for (; n < 9; n++) nano *= 10;
        }
        if (ok && p < end && (*p == 'Z' || *p == 'z')) {
            p++;
        } else if (ok && p < end && (*p == '+' || *p == '-')) {
            int64_t sign = *p++ == '-' ? -1 : 1, oh, om;
            ok = k_digits(&p, end, 2, &oh) && p < end && *p++ == ':' && k_digits(&p, end, 2, &om);
            offset = sign * (oh * 3600 + om * 60);
        } else {
            ok = false;
        }
    }
    int64_t mdays = mo == 2 ? ((y % 4 == 0 && y % 100 != 0) || y % 400 == 0 ? 29 : 28)
                            : (mo == 4 || mo == 6 || mo == 9 || mo == 11 ? 30 : 31);
    ok = ok && p == end && mo >= 1 && mo <= 12 && d >= 1 && d <= mdays && h < 24 && mi < 60 && s < 60;
    if (!ok) {
        k_lib_error = k_concat(k_str("not a valid time: "), k_str_quote(text));
        return 0;
    }
    int64_t secs = k_days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s - offset;
    return secs * K_NS_PER_SEC + nano;
}

/* ----- fs ----- */

K_API kstr *k_fs_read(kstr *path) {
    FILE *f = fopen(k_cstr(path), "rb");
    if (!f) { k_lib_fail("read", path); return k_str(""); }
    size_t cap = 4096, len = 0;
    char *buf = k_alloc_atomic(cap);
    size_t n;
    while ((n = fread(buf + len, 1, cap - len, f)) > 0) {
        len += n;
        if (len == cap) {
            char *b = k_alloc_atomic(cap * 2);
            memcpy(b, buf, len);
            buf = b;
            cap *= 2;
        }
    }
    bool failed = ferror(f);
    fclose(f);
    if (failed) { k_lib_fail("read", path); return k_str(""); }
    return k_str_n(buf, (int64_t)len);
}

K_API klist *k_fs_read_lines(kstr *path) {
    kstr *text = k_fs_read(path);
    klist *lines = k_list(0, NULL);
    if (k_lib_error) return lines;
    int64_t start = 0;
    for (int64_t i = 0; i <= text->len; i++) {
        if (i == text->len || text->data[i] == '\n') {
            if (i == text->len && start == i) break;
            int64_t stop = i > start && text->data[i - 1] == '\r' ? i - 1 : i;
            k_list_push(lines, K_P2KV(k_str_n(text->data + start, stop - start)));
            start = i + 1;
        }
    }
    return lines;
}

K_API void k_fs_put(kstr *path, kstr *text, const char *mode) {
    FILE *f = fopen(k_cstr(path), mode);
    if (!f) { k_lib_fail("write", path); return; }
    size_t n = fwrite(text->data, 1, (size_t)text->len, f);
    if (fclose(f) != 0 || n != (size_t)text->len) k_lib_fail("write", path);
}

K_API void k_fs_write(kstr *path, kstr *text) { k_fs_put(path, text, "wb"); }
K_API void k_fs_append(kstr *path, kstr *text) { k_fs_put(path, text, "ab"); }

K_API bool k_fs_exists(kstr *path) {
    struct stat st;
    return stat(k_cstr(path), &st) == 0;
}

K_API bool k_fs_is_dir(kstr *path) {
    struct stat st;
    return stat(k_cstr(path), &st) == 0 && S_ISDIR(st.st_mode);
}

K_API int k_str_ptr_cmp(const void *a, const void *b) {
    return k_str_cmp(K_KV2P(kstr *, *(const kv *)a), K_KV2P(kstr *, *(const kv *)b));
}

K_API klist *k_fs_list(kstr *path) {
    klist *names = k_list(0, NULL);
    DIR *dir = opendir(k_cstr(path));
    if (!dir) { k_lib_fail("list", path); return names; }
    struct dirent *entry;
    while ((entry = readdir(dir)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0) continue;
        k_list_push(names, K_P2KV(k_str(entry->d_name)));
    }
    closedir(dir);
    qsort(names->items, (size_t)names->len, sizeof(kv), k_str_ptr_cmp);
    return names;
}

K_API void k_fs_mkdir(kstr *path) {
    kstr *p = k_str_n(path->data, path->len);
    for (int64_t i = 1; i <= p->len; i++) {
        if (i < p->len && p->data[i] != '/') continue;
        char saved = p->data[i];
        p->data[i] = 0;
        if (mkdir(p->data, 0777) != 0 && !(errno == EEXIST && k_fs_is_dir(p))) {
            p->data[i] = saved;
            k_lib_fail("create the directory", path);
            return;
        }
        p->data[i] = saved;
    }
}

K_API void k_fs_remove(kstr *path) {
    if (remove(k_cstr(path)) != 0) k_lib_fail("remove", path);
}

K_API void k_fs_rename(kstr *from, kstr *to) {
    if (rename(k_cstr(from), k_cstr(to)) != 0) k_lib_fail("rename", from);
}

/* ----- os ----- */

K_API int k_argc = 0;
K_API char **k_argv = NULL;

K_API klist *k_os_args(void) {
    klist *l = k_list(0, NULL);
    for (int i = 1; i < k_argc; i++) k_list_push(l, K_P2KV(k_str(k_argv[i])));
    return l;
}

K_API kbox *k_os_env(kstr *name) {
    const char *v = getenv(k_cstr(name));
    return v ? k_box(K_P2KV(k_str(v))) : NULL;
}

K_API _Noreturn void k_os_exit(int64_t code) {
    fflush(stdout);
    exit((int)code);
}

K_API kstr *k_os_cwd(void) {
    size_t cap = 256;
    for (;;) {
        char *buf = k_alloc_atomic(cap);
        if (getcwd(buf, cap)) return k_str(buf);
        if (errno != ERANGE) return k_str(".");
        cap *= 2;
    }
}

K_API kstr *k_os_temp_dir(void) {
    const char *v = getenv("TMPDIR");
    if (!v || !*v) return k_str("/tmp");
    size_t n = strlen(v);
    while (n > 1 && v[n - 1] == '/') n--;
    return k_str_n(v, (int64_t)n);
}

K_API int64_t k_os_pid(void) { return (int64_t)getpid(); }

K_API kstr *k_os_platform(void) {
#if defined(__linux__)
    return k_str("linux");
#elif defined(__APPLE__)
    return k_str("macos");
#elif defined(_WIN32)
    return k_str("windows");
#else
    return k_str("unknown");
#endif
}

/* ----- io ----- */

K_API kbox *k_io_read_line(void) {
    size_t cap = 128, len = 0;
    char *buf = k_alloc_atomic(cap);
    int c;
    while ((c = getchar()) != EOF && c != '\n') {
        if (len + 1 == cap) {
            char *b = k_alloc_atomic(cap * 2);
            memcpy(b, buf, len);
            buf = b;
            cap *= 2;
        }
        buf[len++] = (char)c;
    }
    if (c == EOF && len == 0) return NULL;
    if (len > 0 && buf[len - 1] == '\r') len--;
    return k_box(K_P2KV(k_str_n(buf, (int64_t)len)));
}

K_API kstr *k_io_read_all(void) {
    size_t cap = 4096, len = 0, n;
    char *buf = k_alloc_atomic(cap);
    while ((n = fread(buf + len, 1, cap - len, stdin)) > 0) {
        len += n;
        if (len == cap) {
            char *b = k_alloc_atomic(cap * 2);
            memcpy(b, buf, len);
            buf = b;
            cap *= 2;
        }
    }
    return k_str_n(buf, (int64_t)len);
}

K_API klist *k_io_read_lines(void) {
    klist *lines = k_list(0, NULL);
    kbox *line;
    while ((line = k_io_read_line()) != NULL) k_list_push(lines, line->v);
    return lines;
}

K_API void k_io_write(kstr *s) { fwrite(s->data, 1, (size_t)s->len, stdout); }

K_API void k_io_print_err(kstr *s) {
    fflush(stdout);
    fwrite(s->data, 1, (size_t)s->len, stderr);
    fputc('\n', stderr);
}
