/* The Kiln runtime, prepended to every generated C file.
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
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef uint64_t kv;

static inline kv k_f2kv(double f) { kv v; memcpy(&v, &f, sizeof v); return v; }
static inline double k_kv2f(kv v) { double f; memcpy(&f, &v, sizeof f); return f; }
#define K_P2KV(p) ((kv)(uintptr_t)(p))
#define K_KV2P(t, v) ((t)(uintptr_t)(v))

static void *k_alloc(size_t n) {
    void *p = GC_MALLOC(n);
    if (!p) { fputs("panic: out of memory\n", stderr); exit(101); }
    return p;
}

static void *k_alloc_atomic(size_t n) {
    void *p = GC_MALLOC_ATOMIC(n);
    if (!p) { fputs("panic: out of memory\n", stderr); exit(101); }
    return p;
}

/* ----- strings ----- */

typedef struct kstr { int64_t len; char data[]; } kstr;

static kstr *k_str_n(const char *s, int64_t n) {
    kstr *r = k_alloc_atomic(sizeof(kstr) + (size_t)n + 1);
    r->len = n;
    memcpy(r->data, s, (size_t)n);
    r->data[n] = 0;
    return r;
}

static kstr *k_str(const char *s) { return k_str_n(s, (int64_t)strlen(s)); }

static kstr *k_concat(kstr *a, kstr *b) {
    kstr *r = k_alloc_atomic(sizeof(kstr) + (size_t)(a->len + b->len) + 1);
    r->len = a->len + b->len;
    memcpy(r->data, a->data, (size_t)a->len);
    memcpy(r->data + a->len, b->data, (size_t)b->len);
    r->data[r->len] = 0;
    return r;
}

static bool k_str_eq(kstr *a, kstr *b) {
    return a->len == b->len && memcmp(a->data, b->data, (size_t)a->len) == 0;
}

static int k_str_cmp(kstr *a, kstr *b) {
    int64_t n = a->len < b->len ? a->len : b->len;
    int c = memcmp(a->data, b->data, (size_t)n);
    if (c != 0) return c;
    return a->len < b->len ? -1 : a->len > b->len;
}

/* ----- panics and errors ----- */

static _Noreturn void k_panic(kstr *message) {
    fflush(stdout);
    fprintf(stderr, "panic: %s\n", message->data);
    exit(101);
}

static _Noreturn void k_panic_c(const char *message) { k_panic(k_str(message)); }

/* A failing call sets `k_failed` and stores the error; the caller checks the
 * flag at `try` or `catch`. `k_error_show` renders the error if it reaches
 * the top of `main`. */
static bool k_failed = false;
static kv k_error = 0;
static kstr *(*k_error_show)(kv) = NULL;

static void k_fail(kv error, kstr *(*show)(kv)) {
    k_failed = true;
    k_error = error;
    k_error_show = show;
}

static kv k_take_error(void) {
    k_failed = false;
    return k_error;
}

/* ----- numbers ----- */

static int64_t k_add(int64_t a, int64_t b) {
    int64_t r;
    if (__builtin_add_overflow(a, b, &r)) k_panic_c("integer overflow");
    return r;
}
static int64_t k_sub(int64_t a, int64_t b) {
    int64_t r;
    if (__builtin_sub_overflow(a, b, &r)) k_panic_c("integer overflow");
    return r;
}
static int64_t k_mul(int64_t a, int64_t b) {
    int64_t r;
    if (__builtin_mul_overflow(a, b, &r)) k_panic_c("integer overflow");
    return r;
}
static int64_t k_div(int64_t a, int64_t b) {
    if (b == 0) k_panic_c("division by zero");
    if (a == INT64_MIN && b == -1) k_panic_c("integer overflow");
    return a / b;
}
static int64_t k_rem(int64_t a, int64_t b) {
    if (b == 0) k_panic_c("division by zero");
    if (b == -1) return 0;
    return a % b;
}
static int64_t k_neg(int64_t a) {
    if (a == INT64_MIN) k_panic_c("integer overflow");
    return -a;
}

static int64_t k_abs(int64_t a) { return a < 0 ? k_neg(a) : a; }
static int64_t k_min(int64_t a, int64_t b) { return a < b ? a : b; }
static int64_t k_max(int64_t a, int64_t b) { return a > b ? a : b; }

static kstr *k_int_str(int64_t n) {
    char buf[32];
    int len = snprintf(buf, sizeof buf, "%" PRId64, n);
    return k_str_n(buf, len);
}

static kstr *k_float_str(double f) {
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

static kstr *k_bool_str(bool b) { return k_str(b ? "true" : "false"); }

static kstr *k_duration_str(int64_t ns) {
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

static int64_t k_float_to_int(double f) {
    if (!(f >= -9.2233720368547758e18 && f < 9.2233720368547758e18)) k_panic_c("float is out of the Int range");
    return (int64_t)f;
}

/* ----- optionals: none is NULL, a value is boxed as a kv ----- */

typedef struct kbox { kv v; } kbox;

static kbox *k_box(kv v) {
    kbox *b = k_alloc(sizeof(kbox));
    b->v = v;
    return b;
}

/* ----- closures ----- */

typedef struct kclosure { kv (*fn)(void *env, kv *args); void *env; } kclosure;

static kclosure *k_closure(kv (*fn)(void *, kv *), void *env) {
    kclosure *c = k_alloc(sizeof(kclosure));
    c->fn = fn;
    c->env = env;
    return c;
}

static inline kv k_call1(kclosure *c, kv a) { kv args[1] = {a}; return c->fn(c->env, args); }

/* ----- enums ----- */

typedef struct kenum { int64_t tag; kv f[]; } kenum;

static kenum *k_enum(int64_t tag, int n, const kv *fields) {
    kenum *e = k_alloc(sizeof(kenum) + sizeof(kv) * (size_t)n);
    e->tag = tag;
    for (int i = 0; i < n; i++) e->f[i] = fields[i];
    return e;
}

/* ----- lists ----- */

typedef struct klist { int64_t len, cap; kv *items; } klist;

static klist *k_list(int64_t n, const kv *items) {
    klist *l = k_alloc(sizeof(klist));
    l->len = n;
    l->cap = n < 4 ? 4 : n;
    l->items = k_alloc(sizeof(kv) * (size_t)l->cap);
    for (int64_t i = 0; i < n; i++) l->items[i] = items[i];
    return l;
}

static void k_list_push(klist *l, kv item) {
    if (l->len == l->cap) {
        int64_t cap = l->cap * 2;
        kv *items = k_alloc(sizeof(kv) * (size_t)cap);
        memcpy(items, l->items, sizeof(kv) * (size_t)l->len);
        l->items = items;
        l->cap = cap;
    }
    l->items[l->len++] = item;
}

static kv k_list_at(klist *l, int64_t i) {
    if (i < 0 || i >= l->len) {
        char buf[96];
        snprintf(buf, sizeof buf, "index %" PRId64 " is out of range for a list of length %" PRId64, i, l->len);
        k_panic_c(buf);
    }
    return l->items[i];
}

static void k_list_set(klist *l, int64_t i, kv v) {
    k_list_at(l, i);
    l->items[i] = v;
}

static klist *k_range(int64_t lo, int64_t hi) {
    klist *l = k_list(0, NULL);
    for (int64_t i = lo; i < hi; i++) k_list_push(l, (kv)i);
    return l;
}

static kbox *k_list_get(klist *l, int64_t i) { return i >= 0 && i < l->len ? k_box(l->items[i]) : NULL; }
static kbox *k_list_first(klist *l) { return k_list_get(l, 0); }
static kbox *k_list_last(klist *l) { return k_list_get(l, l->len - 1); }

static klist *k_list_filter(klist *l, kclosure *keep) {
    klist *r = k_list(0, NULL);
    for (int64_t i = 0; i < l->len; i++)
        if ((bool)k_call1(keep, l->items[i])) k_list_push(r, l->items[i]);
    return r;
}

static klist *k_list_map(klist *l, kclosure *f) {
    klist *r = k_list(0, NULL);
    for (int64_t i = 0; i < l->len; i++) k_list_push(r, k_call1(f, l->items[i]));
    return r;
}

static kbox *k_list_find(klist *l, kclosure *f) {
    for (int64_t i = 0; i < l->len; i++)
        if ((bool)k_call1(f, l->items[i])) return k_box(l->items[i]);
    return NULL;
}

static bool k_list_any(klist *l, kclosure *f) {
    for (int64_t i = 0; i < l->len; i++)
        if ((bool)k_call1(f, l->items[i])) return true;
    return false;
}

static bool k_list_all(klist *l, kclosure *f) {
    for (int64_t i = 0; i < l->len; i++)
        if (!(bool)k_call1(f, l->items[i])) return false;
    return true;
}

static bool k_list_contains(klist *l, kv item, bool (*eq)(kv, kv)) {
    for (int64_t i = 0; i < l->len; i++)
        if (eq(l->items[i], item)) return true;
    return false;
}

static klist *k_list_reverse(klist *l) {
    klist *r = k_list(0, NULL);
    for (int64_t i = l->len - 1; i >= 0; i--) k_list_push(r, l->items[i]);
    return r;
}

/* A stable merge sort by precomputed keys. */
static klist *k_list_sort_by(klist *l, kclosure *key, int (*cmp)(kv, kv)) {
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

static kstr *k_list_join(klist *l, kstr *sep, kstr *(*show)(kv)) {
    kstr *r = k_str("");
    for (int64_t i = 0; i < l->len; i++) {
        if (i > 0) r = k_concat(r, sep);
        r = k_concat(r, show(l->items[i]));
    }
    return r;
}

/* `[a, b]`, with each item shown by `show`. */
static kstr *k_list_show(klist *l, kstr *(*show)(kv)) {
    kstr *r = k_concat(k_str("["), k_list_join(l, k_str(", "), show));
    return k_concat(r, k_str("]"));
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

static uint64_t k_hash_int(kv v) {
    uint64_t x = v;
    x ^= x >> 33; x *= 0xff51afd7ed558ccdULL;
    x ^= x >> 33; x *= 0xc4ceb9fe1a85ec53ULL;
    return x ^ (x >> 33);
}

static uint64_t k_hash_float(kv v) {
    double f = k_kv2f(v);
    if (f == 0) f = 0; /* -0.0 and 0.0 are equal, so they hash the same */
    return k_hash_int(k_f2kv(f));
}

static uint64_t k_hash_str(kv v) {
    kstr *s = K_KV2P(kstr *, v);
    uint64_t h = 1469598103934665603ULL;
    for (int64_t i = 0; i < s->len; i++) { h ^= (unsigned char)s->data[i]; h *= 1099511628211ULL; }
    return h;
}

static kmap *k_map_new(k_hash_fn hash, k_eq_fn eq) {
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
static int64_t k_map_slot(kmap *m, kv key) {
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
static void k_map_rebuild(kmap *m, int64_t cap) {
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

static void k_map_set(kmap *m, kv key, kv val) {
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

static kbox *k_map_get(kmap *m, kv key) {
    int64_t e = m->index[k_map_slot(m, key)];
    return e >= 0 ? k_box(m->vals[e]) : NULL;
}

static bool k_map_contains(kmap *m, kv key) { return m->index[k_map_slot(m, key)] >= 0; }

static kbox *k_map_remove(kmap *m, kv key) {
    int64_t s = k_map_slot(m, key);
    int64_t e = m->index[s];
    if (e < 0) return NULL;
    kbox *old = k_box(m->vals[e]);
    m->index[s] = -2;
    m->live[e] = false;
    m->len--;
    return old;
}

static klist *k_map_keys(kmap *m) {
    klist *l = k_list(0, NULL);
    for (int64_t i = 0; i < m->used; i++) if (m->live[i]) k_list_push(l, m->keys[i]);
    return l;
}

static klist *k_map_values(kmap *m) {
    klist *l = k_list(0, NULL);
    for (int64_t i = 0; i < m->used; i++) if (m->live[i]) k_list_push(l, m->vals[i]);
    return l;
}

static void k_set_add(kmap *m, kv item) { k_map_set(m, item, 0); }
static bool k_set_remove(kmap *m, kv item) { return k_map_remove(m, item) != NULL; }

/* `{"a": 1, "b": 2}`, or `{1, 2}` for a set when `show_val` is NULL. */
static kstr *k_map_show(kmap *m, kstr *(*show_key)(kv), kstr *(*show_val)(kv)) {
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
static bool k_map_eq(kmap *a, kmap *b, k_eq_fn val_eq) {
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

static int64_t k_str_len(kstr *s) {
    int64_t n = 0;
    for (int64_t i = 0; i < s->len; i++)
        if ((s->data[i] & 0xC0) != 0x80) n++;
    return n;
}

static kstr *k_str_upper(kstr *s) {
    kstr *r = k_str_n(s->data, s->len);
    for (int64_t i = 0; i < r->len; i++)
        if (r->data[i] >= 'a' && r->data[i] <= 'z') r->data[i] -= 32;
    return r;
}

static kstr *k_str_lower(kstr *s) {
    kstr *r = k_str_n(s->data, s->len);
    for (int64_t i = 0; i < r->len; i++)
        if (r->data[i] >= 'A' && r->data[i] <= 'Z') r->data[i] += 32;
    return r;
}

static bool k_is_space(char c) { return c == ' ' || c == '\t' || c == '\n' || c == '\r'; }

static kstr *k_str_trim(kstr *s) {
    int64_t a = 0, b = s->len;
    while (a < b && k_is_space(s->data[a])) a++;
    while (b > a && k_is_space(s->data[b - 1])) b--;
    return k_str_n(s->data + a, b - a);
}

static int64_t k_str_find(kstr *s, kstr *part, int64_t from) {
    if (part->len == 0) return from;
    for (int64_t i = from; i + part->len <= s->len; i++)
        if (memcmp(s->data + i, part->data, (size_t)part->len) == 0) return i;
    return -1;
}

static bool k_str_contains(kstr *s, kstr *part) { return k_str_find(s, part, 0) >= 0; }

static bool k_str_starts_with(kstr *s, kstr *p) {
    return p->len <= s->len && memcmp(s->data, p->data, (size_t)p->len) == 0;
}

static bool k_str_ends_with(kstr *s, kstr *p) {
    return p->len <= s->len && memcmp(s->data + s->len - p->len, p->data, (size_t)p->len) == 0;
}

static kstr *k_str_replace(kstr *s, kstr *from, kstr *to) {
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

static klist *k_str_split(kstr *s, kstr *sep) {
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

static kbox *k_str_to_int(kstr *s) {
    if (s->len == 0) return NULL;
    char *end;
    errno = 0;
    long long n = strtoll(s->data, &end, 10);
    if (errno != 0 || end != s->data + s->len) return NULL;
    return k_box((kv)(int64_t)n);
}

/* `"text"` with quotes and escapes, for strings inside lists and structs. */
static kstr *k_str_quote(kstr *s) {
    kstr *r = k_str("\"");
    for (int64_t i = 0; i < s->len; i++) {
        char c = s->data[i];
        const char *esc = c == '"' ? "\\\"" : c == '\\' ? "\\\\" : c == '\n' ? "\\n" : c == '\t' ? "\\t" : NULL;
        r = esc ? k_concat(r, k_str(esc)) : k_concat(r, k_str_n(&c, 1));
    }
    return k_concat(r, k_str("\""));
}

/* ----- built-in functions ----- */

static void k_print(kstr *s) {
    fwrite(s->data, 1, (size_t)s->len, stdout);
    fputc('\n', stdout);
}

static kstr *k_env(kstr *name) {
    const char *v = getenv(name->data);
    return k_str(v ? v : "");
}

static void k_log(const char *level, kstr *message, int n, kstr **data) {
    fflush(stdout);
    fprintf(stderr, "%s %s", level, message->data);
    for (int i = 0; i < n; i++) fprintf(stderr, " %s", data[i]->data);
    fputc('\n', stderr);
}
