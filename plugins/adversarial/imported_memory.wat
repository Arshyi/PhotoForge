(module
  (import "env" "memory" (memory 1))
  (func (export "pf_abi_version") (result i32) (i32.const 1))
  (func (export "pf_alloc") (param i32) (result i32) (i32.const 16))
  (func (export "pf_filter") (param $f i32) (param $in i32) (param $ix i32) (param $iy i32) (param $iw i32) (param $ih i32)
      (param $out i32) (param $ox i32) (param $oy i32) (param $ow i32) (param $oh i32)
      (param $W i32) (param $H i32) (param $params i32) (param $np i32) (result i32) (i32.const 0)))
