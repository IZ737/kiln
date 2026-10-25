/* The native backend's runtime: the shared runtime in `kiln.c`, compiled as
 * an object of its own, plus what generated C would otherwise provide.
 *
 * The C backend writes a show, eq, hash and cmp function in C for every type
 * that needs one. The native backend instead describes each type with a
 * `kdesc` in the program's data, and these functions walk the description.
 * The code generator makes a one-line function per type where the runtime
 * wants a function pointer, such as a map's hash.
 *
 * Structs, records and enum fields are arrays of `kv` slots: a struct with
 * fields is a `kv *`, an enum a `kenum *`, and a struct without fields is
 * NULL. */
#define K_API
#include "kiln.c"


kstr *k_desc_show(const kdesc *d, kv v, bool quote);
bool k_desc_eq(const kdesc *d, kv a, kv b);

static kstr *k_desc_fields(kstr *r, const kdesc *d, const kv *x) {
    for (int64_t i = 0; i < d->n; i++) {
        if (i > 0) r = k_concat(r, k_str(", "));
        if (d->labels[i]) {
            r = k_concat(r, k_str(d->labels[i]));
            r = k_concat(r, k_str(": "));
        }
        r = k_concat(r, k_desc_show(d->parts[i], x[i], true));
    }
    return r;
}

/* The value as `print` shows it, or as it appears inside a list or struct
 * with `quote`, where strings are quoted. */
kstr *k_desc_show(const kdesc *d, kv v, bool quote) {
    switch (d->kind) {
    case K_D_STR: return quote ? k_str_quote(K_KV2P(kstr *, v)) : K_KV2P(kstr *, v);
    case K_D_INT: return k_int_str((int64_t)v);
    case K_D_FLOAT: return k_float_str(k_kv2f(v));
    case K_D_BOOL: return k_bool_str(v != 0);
    case K_D_DURATION: return k_duration_str((int64_t)v);
    case K_D_OPT: {
        kbox *b = K_KV2P(kbox *, v);
        return b ? k_desc_show(d->parts[0], b->v, quote) : k_str("none");
    }
    case K_D_FN: return k_str("<fn>");
    case K_D_LIST: {
        klist *l = K_KV2P(klist *, v);
        kstr *r = k_str("[");
        for (int64_t i = 0; i < l->len; i++) {
            if (i > 0) r = k_concat(r, k_str(", "));
            r = k_concat(r, k_desc_show(d->parts[0], l->items[i], true));
        }
        return k_concat(r, k_str("]"));
    }
    case K_D_MAP:
    case K_D_SET: {
        kmap *m = K_KV2P(kmap *, v);
        kstr *r = k_str("{");
        bool first = true;
        for (int64_t i = 0; i < m->used; i++) {
            if (!m->live[i]) continue;
            if (!first) r = k_concat(r, k_str(", "));
            first = false;
            r = k_concat(r, k_desc_show(d->parts[0], m->keys[i], true));
            if (d->kind == K_D_MAP) {
                r = k_concat(r, k_str(": "));
                r = k_concat(r, k_desc_show(d->parts[1], m->vals[i], true));
            }
        }
        return k_concat(r, k_str("}"));
    }
    case K_D_STRUCT: {
        if (d->n == 0) return k_str(d->name);
        kstr *r = k_concat(k_str(d->name), k_str("("));
        r = k_desc_fields(r, d, K_KV2P(kv *, v));
        return k_concat(r, k_str(")"));
    }
    case K_D_ENUM: {
        kenum *e = K_KV2P(kenum *, v);
        const kdesc *var = d->parts[e->tag];
        if (var->n == 0) return k_str(var->name);
        kstr *r = k_concat(k_str(var->name), k_str("("));
        r = k_desc_fields(r, var, e->f);
        return k_concat(r, k_str(")"));
    }
    case K_D_RECORD: {
        kstr *r = k_desc_fields(k_str("{ "), d, K_KV2P(kv *, v));
        return k_concat(r, k_str(" }"));
    }
    }
    return k_str("");
}

static bool k_desc_fields_eq(const kdesc *d, const kv *x, const kv *y) {
    for (int64_t i = 0; i < d->n; i++)
        if (!k_desc_eq(d->parts[i], x[i], y[i])) return false;
    return true;
}

/* Deep equality. */
bool k_desc_eq(const kdesc *d, kv a, kv b) {
    switch (d->kind) {
    case K_D_STR: return k_str_eq(K_KV2P(kstr *, a), K_KV2P(kstr *, b));
    case K_D_FLOAT: return k_kv2f(a) == k_kv2f(b);
    case K_D_INT:
    case K_D_BOOL:
    case K_D_DURATION:
    case K_D_FN: return a == b;
    case K_D_OPT: {
        kbox *x = K_KV2P(kbox *, a), *y = K_KV2P(kbox *, b);
        if (!x || !y) return x == y;
        return k_desc_eq(d->parts[0], x->v, y->v);
    }
    case K_D_LIST: {
        klist *x = K_KV2P(klist *, a), *y = K_KV2P(klist *, b);
        if (x->len != y->len) return false;
        for (int64_t i = 0; i < x->len; i++)
            if (!k_desc_eq(d->parts[0], x->items[i], y->items[i])) return false;
        return true;
    }
    case K_D_MAP:
    case K_D_SET: {
        kmap *x = K_KV2P(kmap *, a), *y = K_KV2P(kmap *, b);
        if (x->len != y->len) return false;
        for (int64_t i = 0; i < x->used; i++) {
            if (!x->live[i]) continue;
            int64_t e = y->index[k_map_slot(y, x->keys[i])];
            if (e < 0) return false;
            if (d->kind == K_D_MAP && !k_desc_eq(d->parts[1], x->vals[i], y->vals[e])) return false;
        }
        return true;
    }
    case K_D_STRUCT:
        if (d->n == 0) return true;
        return k_desc_fields_eq(d, K_KV2P(kv *, a), K_KV2P(kv *, b));
    case K_D_RECORD: return k_desc_fields_eq(d, K_KV2P(kv *, a), K_KV2P(kv *, b));
    case K_D_ENUM: {
        kenum *x = K_KV2P(kenum *, a), *y = K_KV2P(kenum *, b);
        if (x->tag != y->tag) return false;
        return k_desc_fields_eq(d->parts[x->tag], x->f, y->f);
    }
    }
    return false;
}

/* A hash for map keys and set items; values that are equal hash the same. */
uint64_t k_desc_hash(const kdesc *d, kv v) {
    switch (d->kind) {
    case K_D_STR: return k_hash_str(v);
    case K_D_INT:
    case K_D_BOOL:
    case K_D_DURATION: return k_hash_int(v);
    case K_D_FLOAT: return k_hash_float(v);
    case K_D_OPT: {
        kbox *x = K_KV2P(kbox *, v);
        return x ? k_hash_combine(1, k_desc_hash(d->parts[0], x->v)) : 0;
    }
    case K_D_LIST: {
        klist *x = K_KV2P(klist *, v);
        uint64_t r = (uint64_t)x->len;
        for (int64_t i = 0; i < x->len; i++) r = k_hash_combine(r, k_desc_hash(d->parts[0], x->items[i]));
        return r;
    }
    case K_D_STRUCT:
    case K_D_RECORD: {
        uint64_t r = 0;
        kv *x = K_KV2P(kv *, v);
        for (int64_t i = 0; i < d->n; i++) r = k_hash_combine(r, k_desc_hash(d->parts[i], x[i]));
        return r;
    }
    case K_D_ENUM: {
        kenum *x = K_KV2P(kenum *, v);
        const kdesc *var = d->parts[x->tag];
        uint64_t r = k_hash_int((kv)x->tag);
        for (int64_t i = 0; i < var->n; i++) r = k_hash_combine(r, k_desc_hash(var->parts[i], x->f[i]));
        return r;
    }
    }
    return 0;
}

int k_desc_cmp(const kdesc *d, kv a, kv b);

static int k_desc_fields_cmp(const kdesc *d, const kv *x, const kv *y) {
    for (int64_t i = 0; i < d->n; i++) {
        int c = k_desc_cmp(d->parts[i], x[i], y[i]);
        if (c != 0) return c;
    }
    return 0;
}

/* Orders values: numbers by value, strings by bytes, `false` before `true`,
 * `none` before any value, lists item by item, structs and records field by
 * field, and enums by variant, then fields. */
int k_desc_cmp(const kdesc *d, kv a, kv b) {
    switch (d->kind) {
    case K_D_STR: return k_str_cmp(K_KV2P(kstr *, a), K_KV2P(kstr *, b));
    case K_D_FLOAT: {
        double x = k_kv2f(a), y = k_kv2f(b);
        return x < y ? -1 : x > y;
    }
    case K_D_INT:
    case K_D_DURATION: {
        int64_t x = (int64_t)a, y = (int64_t)b;
        return x < y ? -1 : x > y;
    }
    case K_D_BOOL: return (int)(a != 0) - (int)(b != 0);
    case K_D_OPT: {
        kbox *x = K_KV2P(kbox *, a), *y = K_KV2P(kbox *, b);
        if (!x || !y) return (x != NULL) - (y != NULL);
        return k_desc_cmp(d->parts[0], x->v, y->v);
    }
    case K_D_LIST: {
        klist *x = K_KV2P(klist *, a), *y = K_KV2P(klist *, b);
        for (int64_t i = 0; i < x->len && i < y->len; i++) {
            int c = k_desc_cmp(d->parts[0], x->items[i], y->items[i]);
            if (c != 0) return c;
        }
        return x->len < y->len ? -1 : x->len > y->len;
    }
    case K_D_STRUCT:
        if (d->n == 0) return 0;
        return k_desc_fields_cmp(d, K_KV2P(kv *, a), K_KV2P(kv *, b));
    case K_D_RECORD: return k_desc_fields_cmp(d, K_KV2P(kv *, a), K_KV2P(kv *, b));
    case K_D_ENUM: {
        kenum *x = K_KV2P(kenum *, a), *y = K_KV2P(kenum *, b);
        if (x->tag != y->tag) return x->tag < y->tag ? -1 : 1;
        return k_desc_fields_cmp(d->parts[x->tag], x->f, y->f);
    }
    }
    return 0;
}

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>

/* On Windows the runtime is an executable linked when `kiln` is built, and
 * `kiln` adds a program to it as new sections (see `pe.rs`). It writes the
 * addresses of the program's `kiln_init` and `kiln_main` here. The table is
 * in `.data`, not `.bss`, so that it has bytes in the file to write. */
typedef struct kiln_program {
    void (*init)(void);
    void (*main)(void);
} kiln_program;
__attribute__((section(".data"))) volatile kiln_program kiln_entry = {0, 0};

/* C library functions that generated code calls, kept in the executable. */
double (*const kiln_keep_math1[])(double) = {round, floor, ceil};
double (*const kiln_keep_math2[])(double, double) = {fmod, fmin, fmax};

#define kiln_init kiln_entry.init
#define kiln_main kiln_entry.main
#else
/* Defined by the generated code: they set the globals and run `main`. */
extern void kiln_init(void);
extern void kiln_main(void);
#endif

int main(int argc, char **argv) {
#ifdef _WIN32
    /* Output is bytes, as on other systems: no `\r` before each `\n`. The
     * executable's manifest makes `argv`, file names and the environment
     * UTF-8, and the console shows UTF-8 output. */
    _setmode(_fileno(stdin), _O_BINARY);
    _setmode(_fileno(stdout), _O_BINARY);
    _setmode(_fileno(stderr), _O_BINARY);
    SetConsoleOutputCP(CP_UTF8);
#else
    /* The collector would read /proc to find the stack, which a minimal
     * system may not have. Kiln code runs in frames below this one. */
    GC_stackbottom = (char *)__builtin_frame_address(0);
#endif
    GC_INIT();
    k_argc = argc;
    k_argv = argv;
    kiln_init();
    kiln_main();
    if (k_failed) {
        fflush(stdout);
        fprintf(stderr, "error: %s\n", k_cstr(k_error_show(k_error)));
        return 1;
    }
    return 0;
}
