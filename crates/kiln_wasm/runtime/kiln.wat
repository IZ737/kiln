  ;; The Kiln runtime for the browser (WasmGC). The backend puts it at the
  ;; start of every module it writes, as the C backend prepends kiln.c.
  ;;
  ;; Values: Int and Duration are i64, Float f64, Bool i32, and a String is
  ;; a JavaScript string (externref), worked on by the functions `kiln.js`
  ;; gives the module. Everything else is a GC reference, null for none.
  ;; Generic code (type parameters, list items, enum fields, closure
  ;; arguments) stores values in slots, which are `anyref`: an Int is boxed
  ;; in a $BoxI64, a Float in a $BoxF64, a Bool is an i31 and a String is
  ;; converted with `any.convert_extern`. An optional `T?` is an $Opt
  ;; holding the value as a slot, or null for none. This is the layout of
  ;; the native runtime (kiln.c), with the browser's collector instead of
  ;; Boehm's.

  (type $Slots (array (mut anyref)))
  (type $I32s (array (mut i32)))
  (type $Bytes (array (mut i8)))
  (type $BoxI64 (struct (field i64)))
  (type $BoxF64 (struct (field f64)))
  (type $Opt (struct (field anyref)))
  (type $Empty (struct))
  (type $Enum (struct (field i32) (field (ref null $Slots))))
  (type $List (struct (field (mut i32)) (field (mut (ref $Slots)))))
  (type $Code (func (param (ref null $Slots)) (param (ref null $Slots)) (result anyref)))
  (type $Closure (struct (field (ref $Code)) (field (ref null $Slots))))
  (type $HashF (func (param anyref) (result i64)))
  (type $EqF (func (param anyref anyref) (result i32)))
  (type $CmpF (func (param anyref anyref) (result i32)))
  (type $ShowF (func (param anyref) (result externref)))
  ;; len, used, cap, keys, vals, live, index, slots, hash, eq
  (type $Map (struct
    (field (mut i32)) (field (mut i32)) (field (mut i32))
    (field (mut (ref $Slots))) (field (mut (ref $Slots))) (field (mut (ref $Bytes)))
    (field (mut (ref $I32s))) (field (mut i32))
    (field (ref $HashF)) (field (ref $EqF))))
  ;; Cells for the `var`s closures capture.
  (type $CellI64 (struct (field (mut i64))))
  (type $CellF64 (struct (field (mut f64))))
  (type $CellI32 (struct (field (mut i32))))
  (type $CellExt (struct (field (mut externref))))
  (type $CellAny (struct (field (mut anyref))))

  (import "k" "print" (func $k_print (param externref)))
  (import "k" "eprint" (func $k_eprint (param externref)))
  (import "k" "fail_exit" (func $k_fail_exit (param externref)))
  (import "k" "panic" (func $k_panic (param externref)))
  (import "k" "panic_code" (func $k_panic_code (param i32 i64 i64)))
  (import "k" "concat" (func $k_concat (param externref externref) (result externref)))
  (import "k" "str_eq" (func $k_str_eq (param externref externref) (result i32)))
  (import "k" "str_cmp" (func $k_str_cmp (param externref externref) (result i32)))
  (import "k" "str_hash" (func $k_str_hash_js (param externref) (result i32)))
  (import "k" "str_len" (func $k_str_len (param externref) (result i64)))
  (import "k" "str_empty" (func $k_str_empty (param externref) (result i32)))
  (import "k" "str_upper" (func $k_str_upper (param externref) (result externref)))
  (import "k" "str_lower" (func $k_str_lower (param externref) (result externref)))
  (import "k" "str_trim" (func $k_str_trim (param externref) (result externref)))
  (import "k" "str_trim_start" (func $k_str_trim_start (param externref) (result externref)))
  (import "k" "str_trim_end" (func $k_str_trim_end (param externref) (result externref)))
  (import "k" "str_contains" (func $k_str_contains (param externref externref) (result i32)))
  (import "k" "str_starts_with" (func $k_str_starts_with (param externref externref) (result i32)))
  (import "k" "str_ends_with" (func $k_str_ends_with (param externref externref) (result i32)))
  (import "k" "str_replace" (func $k_str_replace (param externref externref externref) (result externref)))
  (import "k" "str_repeat" (func $k_str_repeat (param externref i64) (result externref)))
  (import "k" "str_slice" (func $k_str_slice (param externref i64 i64) (result externref)))
  (import "k" "str_pad" (func $k_str_pad (param externref i64 externref i32) (result externref)))
  (import "k" "str_index_of" (func $k_str_index_of (param externref externref) (result i64)))
  (import "k" "str_to_int_ok" (func $k_str_to_int_ok (param externref) (result i32)))
  (import "k" "str_to_int" (func $k_str_to_int (param externref) (result i64)))
  (import "k" "str_to_float_ok" (func $k_str_to_float_ok (param externref) (result i32)))
  (import "k" "str_to_float" (func $k_str_to_float (param externref) (result f64)))
  (import "k" "str_split" (func $k_str_split_js (param externref externref) (result externref)))
  (import "k" "str_chars" (func $k_str_chars_js (param externref) (result externref)))
  (import "k" "str_lines" (func $k_str_lines_js (param externref) (result externref)))
  (import "k" "str_quote" (func $k_str_quote (param externref) (result externref)))
  (import "k" "arr_len" (func $k_arr_len (param externref) (result i32)))
  (import "k" "arr_get" (func $k_arr_get (param externref i32) (result externref)))
  (import "k" "int_str" (func $k_int_str (param i64) (result externref)))
  (import "k" "float_str" (func $k_float_str (param f64) (result externref)))
  (import "k" "float_fixed" (func $k_float_fixed (param f64 i64) (result externref)))
  (import "k" "duration_str" (func $k_duration_str (param i64) (result externref)))
  (import "k" "float_rem" (func $k_float_rem (param f64 f64) (result f64)))
  (import "k" "html_escape" (func $k_html_escape (param externref) (result externref)))
  (import "k" "safe_url" (func $k_safe_url (param externref) (result externref)))
  (import "k" "ui_merge" (func $k_ui_merge (param externref externref) (result externref)))
  (import "k" "base64_encode" (func $k_base64_encode (param externref) (result externref)))
  (import "k" "base64_decode" (func $k_base64_decode (param externref) (result externref)))
  ;; The message of the last JavaScript function that failed, or null.
  (import "k" "take_error" (func $k_take_error_js (result externref)))
  (import "k" "js_array" (func $k_js_array (result externref)))
  (import "k" "js_push" (func $k_js_push (param externref externref)))
  (import "k" "log" (func $k_log (param externref externref externref)))

  ;; ----- definitions -----

  ;; A list of JavaScript values (strings or `js.Value`s, in slots) as a
  ;; JavaScript array.
  (func $k_list_to_js (param $l (ref null $List)) (result externref)
    (local $a externref) (local $i i32)
    (local.set $a (call $k_js_array))
    (block $done
      (loop $next
        (br_if $done (i32.ge_s (local.get $i) (struct.get $List 0 (local.get $l))))
        (call $k_js_push (local.get $a)
          (extern.convert_any (array.get $Slots (struct.get $List 1 (local.get $l)) (local.get $i))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $next)))
    (local.get $a))

  ;; A failing call sets $failed and stores the error; the caller checks
  ;; the flag at `try` or `catch`. $lib_error is the message of a library
  ;; function that failed, which the generated code turns into an `Error`.
  (global $failed (mut i32) (i32.const 0))
  (global $error (mut anyref) (ref.null any))
  (global $error_show (mut (ref null $ShowF)) (ref.null $ShowF))
  (global $lib_error (mut externref) (ref.null extern))

  (func $k_fail (param $e anyref) (param $show (ref null $ShowF))
    (global.set $failed (i32.const 1))
    (global.set $error (local.get $e))
    (global.set $error_show (local.get $show)))

  (func $k_take_error (result anyref)
    (global.set $failed (i32.const 0))
    (global.get $error))

  ;; ----- numbers -----

  (func $k_add (param $a i64) (param $b i64) (result i64)
    (local $r i64)
    (local.set $r (i64.add (local.get $a) (local.get $b)))
    (if (i64.lt_s (i64.and (i64.xor (local.get $a) (local.get $r))
                           (i64.xor (local.get $b) (local.get $r))) (i64.const 0))
      (then (call $k_panic_code (i32.const 1) (i64.const 0) (i64.const 0))))
    (local.get $r))

  (func $k_sub (param $a i64) (param $b i64) (result i64)
    (local $r i64)
    (local.set $r (i64.sub (local.get $a) (local.get $b)))
    (if (i64.lt_s (i64.and (i64.xor (local.get $a) (local.get $b))
                           (i64.xor (local.get $a) (local.get $r))) (i64.const 0))
      (then (call $k_panic_code (i32.const 1) (i64.const 0) (i64.const 0))))
    (local.get $r))

  (func $k_mul (param $a i64) (param $b i64) (result i64)
    (local $r i64)
    (if (i64.eq (local.get $b) (i64.const -1))
      (then
        (if (i64.eq (local.get $a) (i64.const 0x8000000000000000))
          (then (call $k_panic_code (i32.const 1) (i64.const 0) (i64.const 0))))
        (return (i64.sub (i64.const 0) (local.get $a)))))
    (if (i64.eq (local.get $a) (i64.const -1))
      (then
        (if (i64.eq (local.get $b) (i64.const 0x8000000000000000))
          (then (call $k_panic_code (i32.const 1) (i64.const 0) (i64.const 0))))
        (return (i64.sub (i64.const 0) (local.get $b)))))
    (local.set $r (i64.mul (local.get $a) (local.get $b)))
    (if (i64.ne (local.get $b) (i64.const 0))
      (then
        (if (i64.ne (i64.div_s (local.get $r) (local.get $b)) (local.get $a))
          (then (call $k_panic_code (i32.const 1) (i64.const 0) (i64.const 0))))))
    (local.get $r))

  (func $k_div (param $a i64) (param $b i64) (result i64)
    (if (i64.eqz (local.get $b))
      (then (call $k_panic_code (i32.const 2) (i64.const 0) (i64.const 0))))
    (if (i32.and (i64.eq (local.get $a) (i64.const 0x8000000000000000))
                 (i64.eq (local.get $b) (i64.const -1)))
      (then (call $k_panic_code (i32.const 1) (i64.const 0) (i64.const 0))))
    (i64.div_s (local.get $a) (local.get $b)))

  (func $k_rem (param $a i64) (param $b i64) (result i64)
    (if (i64.eqz (local.get $b))
      (then (call $k_panic_code (i32.const 2) (i64.const 0) (i64.const 0))))
    (if (i64.eq (local.get $b) (i64.const -1)) (then (return (i64.const 0))))
    (i64.rem_s (local.get $a) (local.get $b)))

  (func $k_neg (param $a i64) (result i64)
    (if (i64.eq (local.get $a) (i64.const 0x8000000000000000))
      (then (call $k_panic_code (i32.const 1) (i64.const 0) (i64.const 0))))
    (i64.sub (i64.const 0) (local.get $a)))

  (func $k_abs (param $a i64) (result i64)
    (if (result i64) (i64.lt_s (local.get $a) (i64.const 0))
      (then (call $k_neg (local.get $a)))
      (else (local.get $a))))

  (func $k_min (param $a i64) (param $b i64) (result i64)
    (select (local.get $a) (local.get $b) (i64.lt_s (local.get $a) (local.get $b))))

  (func $k_max (param $a i64) (param $b i64) (result i64)
    (select (local.get $a) (local.get $b) (i64.gt_s (local.get $a) (local.get $b))))

  (func $k_pow (param $base i64) (param $exp i64) (result i64)
    (local $r i64)
    (if (i64.lt_s (local.get $exp) (i64.const 0))
      (then (call $k_panic_code (i32.const 3) (i64.const 0) (i64.const 0))))
    (local.set $r (i64.const 1))
    (block $done
      (loop $next
        (br_if $done (i64.le_s (local.get $exp) (i64.const 0)))
        (if (i64.ne (i64.and (local.get $exp) (i64.const 1)) (i64.const 0))
          (then (local.set $r (call $k_mul (local.get $r) (local.get $base)))))
        (local.set $exp (i64.shr_s (local.get $exp) (i64.const 1)))
        (if (i64.gt_s (local.get $exp) (i64.const 0))
          (then (local.set $base (call $k_mul (local.get $base) (local.get $base)))))
        (br $next)))
    (local.get $r))

  (func $k_clamp (param $n i64) (param $lo i64) (param $hi i64) (result i64)
    (if (i64.gt_s (local.get $lo) (local.get $hi))
      (then (call $k_panic_code (i32.const 4) (i64.const 0) (i64.const 0))))
    (if (i64.lt_s (local.get $n) (local.get $lo)) (then (return (local.get $lo))))
    (if (i64.gt_s (local.get $n) (local.get $hi)) (then (return (local.get $hi))))
    (local.get $n))

  (func $k_float_to_int (param $f f64) (result i64)
    (if (i32.eqz (i32.and (f64.ge (local.get $f) (f64.const -9.2233720368547758e18))
                          (f64.lt (local.get $f) (f64.const 9.2233720368547758e18))))
      (then (call $k_panic_code (i32.const 5) (i64.const 0) (i64.const 0))))
    (i64.trunc_f64_s (local.get $f)))

  ;; C's `round`: halfway cases away from zero.
  (func $k_round (param $f f64) (result f64)
    (local $t f64)
    (local.set $t (f64.trunc (local.get $f)))
    (if (f64.ge (f64.abs (f64.sub (local.get $f) (local.get $t))) (f64.const 0.5))
      (then (local.set $t (f64.add (local.get $t) (f64.copysign (f64.const 1) (local.get $f))))))
    (local.get $t))

  ;; C's `fmin` and `fmax`: a NaN loses to a number.
  (func $k_fmin (param $a f64) (param $b f64) (result f64)
    (if (f64.ne (local.get $a) (local.get $a)) (then (return (local.get $b))))
    (if (f64.ne (local.get $b) (local.get $b)) (then (return (local.get $a))))
    (f64.min (local.get $a) (local.get $b)))

  (func $k_fmax (param $a f64) (param $b f64) (result f64)
    (if (f64.ne (local.get $a) (local.get $a)) (then (return (local.get $b))))
    (if (f64.ne (local.get $b) (local.get $b)) (then (return (local.get $a))))
    (f64.max (local.get $a) (local.get $b)))

  (func $k_bool_str (param $b i32) (result externref)
    (if (result externref) (local.get $b)
      (then (global.get $s_true))
      (else (global.get $s_false))))

  ;; ----- slots and boxes -----

  (func $k_box (param $v anyref) (result (ref $Opt))
    (struct.new $Opt (local.get $v)))

  (func $k_i64 (param $v anyref) (result i64)
    (struct.get $BoxI64 0 (ref.cast (ref $BoxI64) (local.get $v))))

  (func $k_f64 (param $v anyref) (result f64)
    (struct.get $BoxF64 0 (ref.cast (ref $BoxF64) (local.get $v))))

  (func $k_i32 (param $v anyref) (result i32)
    (i31.get_u (ref.cast (ref i31) (local.get $v))))

  (func $k_ext (param $v anyref) (result externref)
    (extern.convert_any (local.get $v)))

  ;; ----- closures -----

  (func $k_call1 (param $c (ref null $Closure)) (param $a anyref) (result anyref)
    (call_ref $Code
      (struct.get $Closure 1 (local.get $c))
      (array.new_fixed $Slots 1 (local.get $a))
      (struct.get $Closure 0 (local.get $c))))

  (func $k_call2 (param $c (ref null $Closure)) (param $a anyref) (param $b anyref) (result anyref)
    (call_ref $Code
      (struct.get $Closure 1 (local.get $c))
      (array.new_fixed $Slots 2 (local.get $a) (local.get $b))
      (struct.get $Closure 0 (local.get $c))))

  (func $k_identity (type $Code)
    (array.get $Slots (local.get 1) (i32.const 0)))

  ;; ----- lists -----

  (func $k_list (param $n i32) (result (ref $List))
    (struct.new $List (i32.const 0)
      (array.new $Slots (ref.null any) (select (local.get $n) (i32.const 4) (i32.gt_s (local.get $n) (i32.const 4))))))

  (func $k_len (param $l (ref null $List)) (result i32)
    (struct.get $List 0 (local.get $l)))

  (func $k_list_push (param $l (ref null $List)) (param $v anyref)
    (local $items (ref $Slots))
    (local $len i32)
    (local $new (ref $Slots))
    (local.set $items (struct.get $List 1 (local.get $l)))
    (local.set $len (struct.get $List 0 (local.get $l)))
    (if (i32.eq (local.get $len) (array.len (local.get $items)))
      (then
        (local.set $new (array.new $Slots (ref.null any)
          (select (i32.mul (local.get $len) (i32.const 2)) (i32.const 4)
                  (i32.gt_s (local.get $len) (i32.const 2)))))
        (array.copy $Slots $Slots (local.get $new) (i32.const 0) (local.get $items) (i32.const 0) (local.get $len))
        (struct.set $List 1 (local.get $l) (local.get $new))
        (local.set $items (local.get $new))))
    (array.set $Slots (local.get $items) (local.get $len) (local.get $v))
    (struct.set $List 0 (local.get $l) (i32.add (local.get $len) (i32.const 1))))

  (func $k_list_check (param $l (ref null $List)) (param $i i64)
    (if (i32.or (i64.lt_s (local.get $i) (i64.const 0))
                (i64.ge_s (local.get $i) (i64.extend_i32_s (struct.get $List 0 (local.get $l)))))
      (then (call $k_panic_code (i32.const 6) (local.get $i)
              (i64.extend_i32_s (struct.get $List 0 (local.get $l)))))))

  (func $k_list_at (param $l (ref null $List)) (param $i i64) (result anyref)
    (call $k_list_check (local.get $l) (local.get $i))
    (array.get $Slots (struct.get $List 1 (local.get $l)) (i32.wrap_i64 (local.get $i))))

  (func $k_list_set (param $l (ref null $List)) (param $i i64) (param $v anyref)
    (call $k_list_check (local.get $l) (local.get $i))
    (array.set $Slots (struct.get $List 1 (local.get $l)) (i32.wrap_i64 (local.get $i)) (local.get $v)))

  (func $k_item (param $l (ref null $List)) (param $i i32) (result anyref)
    (array.get $Slots (struct.get $List 1 (local.get $l)) (local.get $i)))

  (func $k_range (param $lo i64) (param $hi i64) (result (ref $List))
    (local $l (ref $List))
    (local.set $l (call $k_list (i32.const 0)))
    (block $done
      (loop $next
        (br_if $done (i64.ge_s (local.get $lo) (local.get $hi)))
        (call $k_list_push (local.get $l) (struct.new $BoxI64 (local.get $lo)))
        (local.set $lo (i64.add (local.get $lo) (i64.const 1)))
        (br $next)))
    (local.get $l))

  (func $k_list_get (param $l (ref null $List)) (param $i i64) (result (ref null $Opt))
    (if (i32.and (i64.ge_s (local.get $i) (i64.const 0))
                 (i64.lt_s (local.get $i) (i64.extend_i32_s (struct.get $List 0 (local.get $l)))))
      (then (return (call $k_box (call $k_item (local.get $l) (i32.wrap_i64 (local.get $i)))))))
    (ref.null $Opt))

  (func $k_list_first (param $l (ref null $List)) (result (ref null $Opt))
    (call $k_list_get (local.get $l) (i64.const 0)))

  (func $k_list_last (param $l (ref null $List)) (result (ref null $Opt))
    (call $k_list_get (local.get $l) (i64.sub (i64.extend_i32_s (struct.get $List 0 (local.get $l))) (i64.const 1))))

  (func $k_list_pop (param $l (ref null $List)) (result (ref null $Opt))
    (local $n i32)
    (local.set $n (struct.get $List 0 (local.get $l)))
    (if (i32.eqz (local.get $n)) (then (return (ref.null $Opt))))
    (local.set $n (i32.sub (local.get $n) (i32.const 1)))
    (struct.set $List 0 (local.get $l) (local.get $n))
    (call $k_box (call $k_item (local.get $l) (local.get $n))))

  (func $k_list_of (param $items (ref $Slots)) (param $from i32) (param $n i32) (result (ref $List))
    (local $l (ref $List))
    (local.set $l (call $k_list (local.get $n)))
    (array.copy $Slots $Slots (struct.get $List 1 (local.get $l)) (i32.const 0)
      (local.get $items) (local.get $from) (local.get $n))
    (struct.set $List 0 (local.get $l) (local.get $n))
    (local.get $l))

  (func $k_list_filter (param $l (ref null $List)) (param $f (ref null $Closure)) (result (ref $List))
    (local $r (ref $List)) (local $i i32) (local $v anyref)
    (local.set $r (call $k_list (i32.const 0)))
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $List 0 (local.get $l))))
      (local.set $v (call $k_item (local.get $l) (local.get $i)))
      (if (call $k_i32 (call $k_call1 (local.get $f) (local.get $v)))
        (then (call $k_list_push (local.get $r) (local.get $v))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (local.get $r))

  (func $k_list_map (param $l (ref null $List)) (param $f (ref null $Closure)) (result (ref $List))
    (local $r (ref $List)) (local $i i32)
    (local.set $r (call $k_list (i32.const 0)))
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $List 0 (local.get $l))))
      (call $k_list_push (local.get $r) (call $k_call1 (local.get $f) (call $k_item (local.get $l) (local.get $i))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (local.get $r))

  (func $k_list_find (param $l (ref null $List)) (param $f (ref null $Closure)) (result (ref null $Opt))
    (local $i i32) (local $v anyref)
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $List 0 (local.get $l))))
      (local.set $v (call $k_item (local.get $l) (local.get $i)))
      (if (call $k_i32 (call $k_call1 (local.get $f) (local.get $v)))
        (then (return (call $k_box (local.get $v)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (ref.null $Opt))

  (func $k_list_any (param $l (ref null $List)) (param $f (ref null $Closure)) (result i32)
    (local $i i32)
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $List 0 (local.get $l))))
      (if (call $k_i32 (call $k_call1 (local.get $f) (call $k_item (local.get $l) (local.get $i))))
        (then (return (i32.const 1))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (i32.const 0))

  (func $k_list_all (param $l (ref null $List)) (param $f (ref null $Closure)) (result i32)
    (local $i i32)
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $List 0 (local.get $l))))
      (if (i32.eqz (call $k_i32 (call $k_call1 (local.get $f) (call $k_item (local.get $l) (local.get $i)))))
        (then (return (i32.const 0))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (i32.const 1))

  (func $k_list_count (param $l (ref null $List)) (param $f (ref null $Closure)) (result i64)
    (local $i i32) (local $n i64)
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $List 0 (local.get $l))))
      (if (call $k_i32 (call $k_call1 (local.get $f) (call $k_item (local.get $l) (local.get $i))))
        (then (local.set $n (i64.add (local.get $n) (i64.const 1)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (local.get $n))

  (func $k_list_fold (param $l (ref null $List)) (param $acc anyref) (param $f (ref null $Closure)) (result anyref)
    (local $i i32)
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $List 0 (local.get $l))))
      (local.set $acc (call $k_call2 (local.get $f) (local.get $acc) (call $k_item (local.get $l) (local.get $i))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (local.get $acc))

  (func $k_list_flat_map (param $l (ref null $List)) (param $f (ref null $Closure)) (result (ref $List))
    (local $r (ref $List)) (local $i i32) (local $j i32) (local $part (ref null $List))
    (local.set $r (call $k_list (i32.const 0)))
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $List 0 (local.get $l))))
      (local.set $part (ref.cast (ref null $List) (call $k_call1 (local.get $f) (call $k_item (local.get $l) (local.get $i)))))
      (local.set $j (i32.const 0))
      (block $pdone (loop $pnext
        (br_if $pdone (i32.ge_s (local.get $j) (struct.get $List 0 (local.get $part))))
        (call $k_list_push (local.get $r) (call $k_item (local.get $part) (local.get $j)))
        (local.set $j (i32.add (local.get $j) (i32.const 1)))
        (br $pnext)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (local.get $r))

  (func $k_list_contains (param $l (ref null $List)) (param $v anyref) (param $eq (ref $EqF)) (result i32)
    (i64.ge_s (call $k_list_find_index (local.get $l) (local.get $v) (local.get $eq)) (i64.const 0)))

  (func $k_list_find_index (param $l (ref null $List)) (param $v anyref) (param $eq (ref $EqF)) (result i64)
    (local $i i32)
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $List 0 (local.get $l))))
      (if (call_ref $EqF (call $k_item (local.get $l) (local.get $i)) (local.get $v) (local.get $eq))
        (then (return (i64.extend_i32_s (local.get $i)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (i64.const -1))

  (func $k_list_index_of (param $l (ref null $List)) (param $v anyref) (param $eq (ref $EqF)) (result (ref null $Opt))
    (local $i i64)
    (local.set $i (call $k_list_find_index (local.get $l) (local.get $v) (local.get $eq)))
    (if (i64.lt_s (local.get $i) (i64.const 0)) (then (return (ref.null $Opt))))
    (call $k_box (struct.new $BoxI64 (local.get $i))))

  (func $k_list_reverse (param $l (ref null $List)) (result (ref $List))
    (local $r (ref $List)) (local $i i32)
    (local.set $r (call $k_list (struct.get $List 0 (local.get $l))))
    (local.set $i (struct.get $List 0 (local.get $l)))
    (block $done (loop $next
      (br_if $done (i32.le_s (local.get $i) (i32.const 0)))
      (local.set $i (i32.sub (local.get $i) (i32.const 1)))
      (call $k_list_push (local.get $r) (call $k_item (local.get $l) (local.get $i)))
      (br $next)))
    (local.get $r))

  (func $k_list_insert (param $l (ref null $List)) (param $i i64) (param $v anyref)
    (local $n i32) (local $items (ref $Slots))
    (local.set $n (struct.get $List 0 (local.get $l)))
    (if (i32.or (i64.lt_s (local.get $i) (i64.const 0)) (i64.gt_s (local.get $i) (i64.extend_i32_s (local.get $n))))
      (then (call $k_panic_code (i32.const 7) (local.get $i) (i64.extend_i32_s (local.get $n)))))
    (call $k_list_push (local.get $l) (local.get $v))
    (local.set $items (struct.get $List 1 (local.get $l)))
    (array.copy $Slots $Slots (local.get $items) (i32.add (i32.wrap_i64 (local.get $i)) (i32.const 1))
      (local.get $items) (i32.wrap_i64 (local.get $i)) (i32.sub (local.get $n) (i32.wrap_i64 (local.get $i))))
    (array.set $Slots (local.get $items) (i32.wrap_i64 (local.get $i)) (local.get $v)))

  (func $k_list_remove_at (param $l (ref null $List)) (param $i i64) (result anyref)
    (local $v anyref) (local $n i32) (local $items (ref $Slots))
    (local.set $v (call $k_list_at (local.get $l) (local.get $i)))
    (local.set $n (struct.get $List 0 (local.get $l)))
    (local.set $items (struct.get $List 1 (local.get $l)))
    (array.copy $Slots $Slots (local.get $items) (i32.wrap_i64 (local.get $i))
      (local.get $items) (i32.add (i32.wrap_i64 (local.get $i)) (i32.const 1))
      (i32.sub (i32.sub (local.get $n) (i32.const 1)) (i32.wrap_i64 (local.get $i))))
    (struct.set $List 0 (local.get $l) (i32.sub (local.get $n) (i32.const 1)))
    (local.get $v))

  ;; Items from `start` up to `end`, both clamped to the list.
  (func $k_list_slice (param $l (ref null $List)) (param $start i64) (param $end i64) (result (ref $List))
    (local $n i64)
    (local.set $n (i64.extend_i32_s (struct.get $List 0 (local.get $l))))
    (if (i64.lt_s (local.get $start) (i64.const 0)) (then (local.set $start (i64.const 0))))
    (if (i64.gt_s (local.get $end) (local.get $n)) (then (local.set $end (local.get $n))))
    (if (i64.lt_s (local.get $end) (local.get $start)) (then (local.set $end (local.get $start))))
    (if (i64.gt_s (local.get $start) (local.get $n)) (then (local.set $start (local.get $n)) (local.set $end (local.get $n))))
    (call $k_list_of (struct.get $List 1 (local.get $l)) (i32.wrap_i64 (local.get $start))
      (i32.wrap_i64 (i64.sub (local.get $end) (local.get $start)))))

  (func $k_list_take (param $l (ref null $List)) (param $n i64) (result (ref $List))
    (call $k_list_slice (local.get $l) (i64.const 0) (local.get $n)))

  (func $k_list_skip (param $l (ref null $List)) (param $n i64) (result (ref $List))
    (call $k_list_slice (local.get $l) (local.get $n) (i64.extend_i32_s (struct.get $List 0 (local.get $l)))))

  (func $k_list_concat (param $a (ref null $List)) (param $b (ref null $List)) (result (ref $List))
    (local $r (ref $List)) (local $i i32)
    (local.set $r (call $k_list_of (struct.get $List 1 (local.get $a)) (i32.const 0) (struct.get $List 0 (local.get $a))))
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $List 0 (local.get $b))))
      (call $k_list_push (local.get $r) (call $k_item (local.get $b) (local.get $i)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (local.get $r))

  ;; A stable merge sort by precomputed keys, as in kiln.c.
  (func $k_list_sort_by (param $l (ref null $List)) (param $key (ref null $Closure)) (param $cmp (ref $CmpF)) (result (ref $List))
    (local $n i32) (local $keys (ref $Slots)) (local $vals (ref $Slots)) (local $tk (ref $Slots)) (local $tv (ref $Slots))
    (local $t (ref $Slots)) (local $w i32) (local $lo i32) (local $mid i32) (local $hi i32)
    (local $i i32) (local $j i32) (local $k i32)
    (local.set $n (struct.get $List 0 (local.get $l)))
    (local.set $keys (array.new $Slots (ref.null any) (local.get $n)))
    (local.set $vals (array.new $Slots (ref.null any) (local.get $n)))
    (local.set $tk (array.new $Slots (ref.null any) (local.get $n)))
    (local.set $tv (array.new $Slots (ref.null any) (local.get $n)))
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (local.get $n)))
      (array.set $Slots (local.get $vals) (local.get $i) (call $k_item (local.get $l) (local.get $i)))
      (array.set $Slots (local.get $keys) (local.get $i) (call $k_call1 (local.get $key) (call $k_item (local.get $l) (local.get $i))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (local.set $w (i32.const 1))
    (block $wdone (loop $wnext
      (br_if $wdone (i32.ge_s (local.get $w) (local.get $n)))
      (local.set $lo (i32.const 0))
      (block $ldone (loop $lnext
        (br_if $ldone (i32.ge_s (local.get $lo) (local.get $n)))
        (local.set $mid (select (i32.add (local.get $lo) (local.get $w)) (local.get $n)
          (i32.lt_s (i32.add (local.get $lo) (local.get $w)) (local.get $n))))
        (local.set $hi (select (i32.add (local.get $lo) (i32.mul (local.get $w) (i32.const 2))) (local.get $n)
          (i32.lt_s (i32.add (local.get $lo) (i32.mul (local.get $w) (i32.const 2))) (local.get $n))))
        (local.set $i (local.get $lo))
        (local.set $j (local.get $mid))
        (local.set $k (local.get $lo))
        (block $mdone (loop $mnext
          (br_if $mdone (i32.eqz (i32.and (i32.lt_s (local.get $i) (local.get $mid)) (i32.lt_s (local.get $j) (local.get $hi)))))
          (if (i32.lt_s (call_ref $CmpF (array.get $Slots (local.get $keys) (local.get $j))
                                        (array.get $Slots (local.get $keys) (local.get $i)) (local.get $cmp)) (i32.const 0))
            (then
              (array.set $Slots (local.get $tk) (local.get $k) (array.get $Slots (local.get $keys) (local.get $j)))
              (array.set $Slots (local.get $tv) (local.get $k) (array.get $Slots (local.get $vals) (local.get $j)))
              (local.set $j (i32.add (local.get $j) (i32.const 1))))
            (else
              (array.set $Slots (local.get $tk) (local.get $k) (array.get $Slots (local.get $keys) (local.get $i)))
              (array.set $Slots (local.get $tv) (local.get $k) (array.get $Slots (local.get $vals) (local.get $i)))
              (local.set $i (i32.add (local.get $i) (i32.const 1)))))
          (local.set $k (i32.add (local.get $k) (i32.const 1)))
          (br $mnext)))
        (block $adone (loop $anext
          (br_if $adone (i32.ge_s (local.get $i) (local.get $mid)))
          (array.set $Slots (local.get $tk) (local.get $k) (array.get $Slots (local.get $keys) (local.get $i)))
          (array.set $Slots (local.get $tv) (local.get $k) (array.get $Slots (local.get $vals) (local.get $i)))
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (local.set $k (i32.add (local.get $k) (i32.const 1)))
          (br $anext)))
        (block $bdone (loop $bnext
          (br_if $bdone (i32.ge_s (local.get $j) (local.get $hi)))
          (array.set $Slots (local.get $tk) (local.get $k) (array.get $Slots (local.get $keys) (local.get $j)))
          (array.set $Slots (local.get $tv) (local.get $k) (array.get $Slots (local.get $vals) (local.get $j)))
          (local.set $j (i32.add (local.get $j) (i32.const 1)))
          (local.set $k (i32.add (local.get $k) (i32.const 1)))
          (br $bnext)))
        (local.set $lo (i32.add (local.get $lo) (i32.mul (local.get $w) (i32.const 2))))
        (br $lnext)))
      (local.set $t (local.get $keys)) (local.set $keys (local.get $tk)) (local.set $tk (local.get $t))
      (local.set $t (local.get $vals)) (local.set $vals (local.get $tv)) (local.set $tv (local.get $t))
      (local.set $w (i32.mul (local.get $w) (i32.const 2)))
      (br $wnext)))
    (call $k_list_of (local.get $vals) (i32.const 0) (local.get $n)))

  (func $k_list_sort (param $l (ref null $List)) (param $cmp (ref $CmpF)) (result (ref $List))
    (call $k_list_sort_by (local.get $l)
      (struct.new $Closure (ref.func $k_identity) (ref.null $Slots)) (local.get $cmp)))

  ;; The smallest item, or with `sign` -1 the largest; the first one on a tie.
  (func $k_list_extreme (param $l (ref null $List)) (param $cmp (ref $CmpF)) (param $sign i32) (result (ref null $Opt))
    (local $best anyref) (local $i i32) (local $v anyref)
    (if (i32.eqz (struct.get $List 0 (local.get $l))) (then (return (ref.null $Opt))))
    (local.set $best (call $k_item (local.get $l) (i32.const 0)))
    (local.set $i (i32.const 1))
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $List 0 (local.get $l))))
      (local.set $v (call $k_item (local.get $l) (local.get $i)))
      (if (i32.lt_s (i32.mul (local.get $sign) (call_ref $CmpF (local.get $v) (local.get $best) (local.get $cmp))) (i32.const 0))
        (then (local.set $best (local.get $v))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (call $k_box (local.get $best)))

  (func $k_list_join (param $l (ref null $List)) (param $sep externref) (param $show (ref $ShowF)) (result externref)
    (local $r externref) (local $i i32)
    (local.set $r (global.get $s_empty))
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $List 0 (local.get $l))))
      (if (i32.gt_s (local.get $i) (i32.const 0))
        (then (local.set $r (call $k_concat (local.get $r) (local.get $sep)))))
      (local.set $r (call $k_concat (local.get $r) (call_ref $ShowF (call $k_item (local.get $l) (local.get $i)) (local.get $show))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (local.get $r))

  (func $k_list_show (param $l (ref null $List)) (param $show (ref $ShowF)) (result externref)
    (call $k_concat
      (call $k_concat (global.get $s_lbracket) (call $k_list_join (local.get $l) (global.get $s_comma) (local.get $show)))
      (global.get $s_rbracket)))

  ;; A list of the strings in a JavaScript array.
  (func $k_list_of_js (param $a externref) (result (ref $List))
    (local $r (ref $List)) (local $i i32) (local $n i32)
    (local.set $n (call $k_arr_len (local.get $a)))
    (local.set $r (call $k_list (local.get $n)))
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (local.get $n)))
      (call $k_list_push (local.get $r) (any.convert_extern (call $k_arr_get (local.get $a) (local.get $i))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (local.get $r))

  ;; ----- strings -----

  (func $k_str_hash (type $HashF)
    (i64.extend_i32_u (call $k_str_hash_js (extern.convert_any (local.get 0)))))

  (func $k_str_to_int_opt (param $s externref) (result (ref null $Opt))
    (if (i32.eqz (call $k_str_to_int_ok (local.get $s))) (then (return (ref.null $Opt))))
    (call $k_box (struct.new $BoxI64 (call $k_str_to_int (local.get $s)))))

  (func $k_str_to_float_opt (param $s externref) (result (ref null $Opt))
    (if (i32.eqz (call $k_str_to_float_ok (local.get $s))) (then (return (ref.null $Opt))))
    (call $k_box (struct.new $BoxF64 (call $k_str_to_float (local.get $s)))))

  (func $k_str_index_of_opt (param $s externref) (param $p externref) (result (ref null $Opt))
    (local $i i64)
    (local.set $i (call $k_str_index_of (local.get $s) (local.get $p)))
    (if (i64.lt_s (local.get $i) (i64.const 0)) (then (return (ref.null $Opt))))
    (call $k_box (struct.new $BoxI64 (local.get $i))))

  ;; ----- maps and sets -----
  ;;
  ;; A hash map from slot to slot that keeps keys in insertion order, as in
  ;; kiln.c: entries live in arrays in the order they were added, and
  ;; `index` is an open-addressing table of entry positions (-1 empty, -2
  ;; deleted). A set is a map whose values are unused.

  (func $k_hash_mix (param $x i64) (result i64)
    (local.set $x (i64.xor (local.get $x) (i64.shr_u (local.get $x) (i64.const 33))))
    (local.set $x (i64.mul (local.get $x) (i64.const 0xff51afd7ed558ccd)))
    (local.set $x (i64.xor (local.get $x) (i64.shr_u (local.get $x) (i64.const 33))))
    (local.set $x (i64.mul (local.get $x) (i64.const 0xc4ceb9fe1a85ec53)))
    (i64.xor (local.get $x) (i64.shr_u (local.get $x) (i64.const 33))))

  (func $k_hash_combine (param $h i64) (param $x i64) (result i64)
    (call $k_hash_mix (i64.xor (local.get $h)
      (i64.add (i64.add (i64.add (local.get $x) (i64.const 0x9e3779b97f4a7c15))
                        (i64.shl (local.get $h) (i64.const 6)))
               (i64.shr_u (local.get $h) (i64.const 2))))))

  (func $k_hash_int (type $HashF)
    (call $k_hash_mix (call $k_i64 (local.get 0))))

  (func $k_hash_bool (type $HashF)
    (call $k_hash_mix (i64.extend_i32_u (call $k_i32 (local.get 0)))))

  (func $k_hash_float (type $HashF)
    (local $f f64)
    (local.set $f (call $k_f64 (local.get 0)))
    (if (f64.eq (local.get $f) (f64.const 0)) (then (local.set $f (f64.const 0))))
    (call $k_hash_mix (i64.reinterpret_f64 (local.get $f))))

  (func $k_map_new (param $hash (ref $HashF)) (param $eq (ref $EqF)) (result (ref $Map))
    (struct.new $Map (i32.const 0) (i32.const 0) (i32.const 8)
      (array.new $Slots (ref.null any) (i32.const 8))
      (array.new $Slots (ref.null any) (i32.const 8))
      (array.new $Bytes (i32.const 0) (i32.const 8))
      (array.new $I32s (i32.const -1) (i32.const 16))
      (i32.const 16)
      (local.get $hash) (local.get $eq)))

  ;; The index slot holding `key`, or the empty slot where it would go.
  (func $k_map_slot (param $m (ref null $Map)) (param $key anyref) (result i32)
    (local $mask i32) (local $i i32) (local $tomb i32) (local $e i32)
    (local.set $mask (i32.sub (struct.get $Map 7 (local.get $m)) (i32.const 1)))
    (local.set $i (i32.and (i32.wrap_i64 (call_ref $HashF (local.get $key) (struct.get $Map 8 (local.get $m))))
                           (local.get $mask)))
    (local.set $tomb (i32.const -1))
    (loop $next
      (local.set $e (array.get $I32s (struct.get $Map 6 (local.get $m)) (local.get $i)))
      (if (i32.eq (local.get $e) (i32.const -1))
        (then (return (select (local.get $tomb) (local.get $i) (i32.ge_s (local.get $tomb) (i32.const 0))))))
      (if (i32.eq (local.get $e) (i32.const -2))
        (then (if (i32.lt_s (local.get $tomb) (i32.const 0)) (then (local.set $tomb (local.get $i)))))
        (else
          (if (call_ref $EqF (array.get $Slots (struct.get $Map 3 (local.get $m)) (local.get $e)) (local.get $key)
                (struct.get $Map 9 (local.get $m)))
            (then (return (local.get $i))))))
      (local.set $i (i32.and (i32.add (local.get $i) (i32.const 1)) (local.get $mask)))
      (br $next))
    (unreachable))

  (func $k_map_entry (param $m (ref null $Map)) (param $key anyref) (result i32)
    (array.get $I32s (struct.get $Map 6 (local.get $m)) (call $k_map_slot (local.get $m) (local.get $key))))

  (func $k_map_rebuild (param $m (ref null $Map)) (param $cap i32)
    (local $keys (ref $Slots)) (local $vals (ref $Slots)) (local $live (ref $Bytes))
    (local $n i32) (local $i i32) (local $slots i32)
    (local.set $keys (array.new $Slots (ref.null any) (local.get $cap)))
    (local.set $vals (array.new $Slots (ref.null any) (local.get $cap)))
    (local.set $live (array.new $Bytes (i32.const 0) (local.get $cap)))
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $Map 1 (local.get $m))))
      (if (array.get_u $Bytes (struct.get $Map 5 (local.get $m)) (local.get $i))
        (then
          (array.set $Slots (local.get $keys) (local.get $n) (array.get $Slots (struct.get $Map 3 (local.get $m)) (local.get $i)))
          (array.set $Slots (local.get $vals) (local.get $n) (array.get $Slots (struct.get $Map 4 (local.get $m)) (local.get $i)))
          (array.set $Bytes (local.get $live) (local.get $n) (i32.const 1))
          (local.set $n (i32.add (local.get $n) (i32.const 1)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (struct.set $Map 3 (local.get $m) (local.get $keys))
    (struct.set $Map 4 (local.get $m) (local.get $vals))
    (struct.set $Map 5 (local.get $m) (local.get $live))
    (struct.set $Map 1 (local.get $m) (local.get $n))
    (struct.set $Map 2 (local.get $m) (local.get $cap))
    (local.set $slots (i32.const 16))
    (block $sdone (loop $snext
      (br_if $sdone (i32.ge_s (local.get $slots) (i32.mul (local.get $cap) (i32.const 2))))
      (local.set $slots (i32.mul (local.get $slots) (i32.const 2)))
      (br $snext)))
    (struct.set $Map 7 (local.get $m) (local.get $slots))
    (struct.set $Map 6 (local.get $m) (array.new $I32s (i32.const -1) (local.get $slots)))
    (local.set $i (i32.const 0))
    (block $idone (loop $inext
      (br_if $idone (i32.ge_s (local.get $i) (local.get $n)))
      (array.set $I32s (struct.get $Map 6 (local.get $m))
        (call $k_map_slot (local.get $m) (array.get $Slots (local.get $keys) (local.get $i)))
        (local.get $i))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $inext))))

  (func $k_map_set (param $m (ref null $Map)) (param $key anyref) (param $val anyref)
    (local $s i32) (local $e i32)
    (local.set $s (call $k_map_slot (local.get $m) (local.get $key)))
    (local.set $e (array.get $I32s (struct.get $Map 6 (local.get $m)) (local.get $s)))
    (if (i32.ge_s (local.get $e) (i32.const 0))
      (then
        (array.set $Slots (struct.get $Map 4 (local.get $m)) (local.get $e) (local.get $val))
        (return)))
    (if (i32.eq (struct.get $Map 1 (local.get $m)) (struct.get $Map 2 (local.get $m)))
      (then
        (call $k_map_rebuild (local.get $m)
          (select (i32.mul (struct.get $Map 0 (local.get $m)) (i32.const 2)) (i32.const 8)
            (i32.gt_s (i32.mul (struct.get $Map 0 (local.get $m)) (i32.const 2)) (i32.const 8))))
        (local.set $s (call $k_map_slot (local.get $m) (local.get $key)))))
    (local.set $e (struct.get $Map 1 (local.get $m)))
    (struct.set $Map 1 (local.get $m) (i32.add (local.get $e) (i32.const 1)))
    (array.set $Slots (struct.get $Map 3 (local.get $m)) (local.get $e) (local.get $key))
    (array.set $Slots (struct.get $Map 4 (local.get $m)) (local.get $e) (local.get $val))
    (array.set $Bytes (struct.get $Map 5 (local.get $m)) (local.get $e) (i32.const 1))
    (array.set $I32s (struct.get $Map 6 (local.get $m)) (local.get $s) (local.get $e))
    (struct.set $Map 0 (local.get $m) (i32.add (struct.get $Map 0 (local.get $m)) (i32.const 1))))

  (func $k_map_get (param $m (ref null $Map)) (param $key anyref) (result (ref null $Opt))
    (local $e i32)
    (local.set $e (call $k_map_entry (local.get $m) (local.get $key)))
    (if (i32.lt_s (local.get $e) (i32.const 0)) (then (return (ref.null $Opt))))
    (call $k_box (array.get $Slots (struct.get $Map 4 (local.get $m)) (local.get $e))))

  (func $k_map_at (param $m (ref null $Map)) (param $key anyref) (result anyref)
    (local $e i32)
    (local.set $e (call $k_map_entry (local.get $m) (local.get $key)))
    (if (i32.lt_s (local.get $e) (i32.const 0))
      (then (call $k_panic_code (i32.const 8) (i64.const 0) (i64.const 0))))
    (array.get $Slots (struct.get $Map 4 (local.get $m)) (local.get $e)))

  (func $k_map_contains (param $m (ref null $Map)) (param $key anyref) (result i32)
    (i32.ge_s (call $k_map_entry (local.get $m) (local.get $key)) (i32.const 0)))

  (func $k_map_remove (param $m (ref null $Map)) (param $key anyref) (result (ref null $Opt))
    (local $s i32) (local $e i32) (local $old (ref null $Opt))
    (local.set $s (call $k_map_slot (local.get $m) (local.get $key)))
    (local.set $e (array.get $I32s (struct.get $Map 6 (local.get $m)) (local.get $s)))
    (if (i32.lt_s (local.get $e) (i32.const 0)) (then (return (ref.null $Opt))))
    (local.set $old (call $k_box (array.get $Slots (struct.get $Map 4 (local.get $m)) (local.get $e))))
    (array.set $I32s (struct.get $Map 6 (local.get $m)) (local.get $s) (i32.const -2))
    (array.set $Bytes (struct.get $Map 5 (local.get $m)) (local.get $e) (i32.const 0))
    (struct.set $Map 0 (local.get $m) (i32.sub (struct.get $Map 0 (local.get $m)) (i32.const 1)))
    (local.get $old))

  (func $k_set_add (param $m (ref null $Map)) (param $v anyref)
    (call $k_map_set (local.get $m) (local.get $v) (ref.null any)))

  (func $k_set_remove (param $m (ref null $Map)) (param $v anyref) (result i32)
    (i32.eqz (ref.is_null (call $k_map_remove (local.get $m) (local.get $v)))))

  ;; The keys (`which` 0), values (1) or `{ key, value }` records (2), in
  ;; insertion order.
  (func $k_map_list (param $m (ref null $Map)) (param $which i32) (result (ref $List))
    (local $r (ref $List)) (local $i i32)
    (local.set $r (call $k_list (struct.get $Map 0 (local.get $m))))
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $Map 1 (local.get $m))))
      (if (array.get_u $Bytes (struct.get $Map 5 (local.get $m)) (local.get $i))
        (then
          (call $k_list_push (local.get $r)
            (if (result anyref) (i32.eqz (local.get $which))
              (then (array.get $Slots (struct.get $Map 3 (local.get $m)) (local.get $i)))
              (else (if (result anyref) (i32.eq (local.get $which) (i32.const 1))
                (then (array.get $Slots (struct.get $Map 4 (local.get $m)) (local.get $i)))
                (else (array.new_fixed $Slots 2
                  (array.get $Slots (struct.get $Map 3 (local.get $m)) (local.get $i))
                  (array.get $Slots (struct.get $Map 4 (local.get $m)) (local.get $i))))))))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (local.get $r))

  ;; `{"a": 1, "b": 2}`, or `{1, 2}` for a set when `show_val` is null.
  (func $k_map_show (param $m (ref null $Map)) (param $sk (ref $ShowF)) (param $sv (ref null $ShowF)) (result externref)
    (local $r externref) (local $i i32) (local $first i32)
    (local.set $r (global.get $s_lbrace))
    (local.set $first (i32.const 1))
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $Map 1 (local.get $m))))
      (if (array.get_u $Bytes (struct.get $Map 5 (local.get $m)) (local.get $i))
        (then
          (if (i32.eqz (local.get $first)) (then (local.set $r (call $k_concat (local.get $r) (global.get $s_comma)))))
          (local.set $first (i32.const 0))
          (local.set $r (call $k_concat (local.get $r)
            (call_ref $ShowF (array.get $Slots (struct.get $Map 3 (local.get $m)) (local.get $i)) (local.get $sk))))
          (if (i32.eqz (ref.is_null (local.get $sv)))
            (then
              (local.set $r (call $k_concat (local.get $r) (global.get $s_colon)))
              (local.set $r (call $k_concat (local.get $r)
                (call_ref $ShowF (array.get $Slots (struct.get $Map 4 (local.get $m)) (local.get $i))
                  (ref.as_non_null (local.get $sv)))))))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (call $k_concat (local.get $r) (global.get $s_rbrace)))

  ;; Maps are equal when they hold the same keys with equal values, in any
  ;; order; sets when they hold the same items.
  (func $k_map_eq (param $a (ref null $Map)) (param $b (ref null $Map)) (param $veq (ref null $EqF)) (result i32)
    (local $i i32) (local $e i32)
    (if (i32.ne (struct.get $Map 0 (local.get $a)) (struct.get $Map 0 (local.get $b))) (then (return (i32.const 0))))
    (block $done (loop $next
      (br_if $done (i32.ge_s (local.get $i) (struct.get $Map 1 (local.get $a))))
      (if (array.get_u $Bytes (struct.get $Map 5 (local.get $a)) (local.get $i))
        (then
          (local.set $e (call $k_map_entry (local.get $b) (array.get $Slots (struct.get $Map 3 (local.get $a)) (local.get $i))))
          (if (i32.lt_s (local.get $e) (i32.const 0)) (then (return (i32.const 0))))
          (if (i32.eqz (ref.is_null (local.get $veq)))
            (then
              (if (i32.eqz (call_ref $EqF (array.get $Slots (struct.get $Map 4 (local.get $a)) (local.get $i))
                                           (array.get $Slots (struct.get $Map 4 (local.get $b)) (local.get $e))
                                           (ref.as_non_null (local.get $veq))))
                (then (return (i32.const 0))))))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $next)))
    (i32.const 1))

  (func $k_eq_ref (type $EqF)
    (ref.eq (ref.cast (ref null eq) (local.get 0)) (ref.cast (ref null eq) (local.get 1))))
