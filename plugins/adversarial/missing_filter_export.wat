(module
  (import "photoforge" "log" (func $log (param i32 i32 i32)))
  (memory (export "memory") 32)
  (global $heap (mut i32) (i32.const 4096))
  (func (export "pf_abi_version") (result i32) (i32.const 1))
  ;; A bump allocator. Nothing is ever freed: every call gets a fresh instance.
  (func (export "pf_alloc") (param $n i32) (result i32)
    (local $p i32) (local $end i32) (local $have i32)
    (local.set $p (i32.and (i32.add (global.get $heap) (i32.const 15)) (i32.const -16)))
    (local.set $end (i32.add (local.get $p) (local.get $n)))
    (local.set $have (i32.shl (memory.size) (i32.const 16)))
    (if (i32.gt_u (local.get $end) (local.get $have))
      (then
        (if (i32.eq
              (memory.grow (i32.shr_u (i32.add (i32.sub (local.get $end) (local.get $have)) (i32.const 65535)) (i32.const 16)))
              (i32.const -1))
          (then (return (i32.const 0))))))
    (global.set $heap (local.get $end))
    (local.get $p))
)
