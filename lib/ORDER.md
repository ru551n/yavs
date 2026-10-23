# Analysis order

Dependency order for analyzing the vendored sources, one file per line as
`<library> <path relative to lib/>`. The reference simulator accepts exactly this order
(`tools/check-sim1.sh`, results in `lib/README.md`). yavs's on-demand analysis
tests read the fenced blocks below, so keep the `order-<std>` info strings.
A package declaration always precedes its body. Files on the same indentation
level do not depend on each other.

## VHDL-1993

```order-1993
std std/1993/standard.vhdl
std std/1993/textio.vhdl
ieee ieee/1993/std_logic_1164.vhdl
ieee ieee/1993/std_logic_1164-body.vhdl
ieee ieee/1993/std_logic_textio.vhdl
ieee ieee/1993/std_logic_textio-body.vhdl
ieee ieee/1993/numeric_std.vhdl
ieee ieee/1993/numeric_std-body.vhdl
ieee ieee/1993/numeric_bit.vhdl
ieee ieee/1993/numeric_bit-body.vhdl
ieee ieee/1993/math_real.vhdl
ieee ieee/1993/math_real-body.vhdl
ieee ieee/1993/math_complex.vhdl
ieee ieee/1993/math_complex-body.vhdl
ieee ieee/1993/std_logic_arith.vhdl
ieee ieee/1993/std_logic_arith-body.vhdl
ieee ieee/1993/std_logic_unsigned.vhdl
ieee ieee/1993/std_logic_unsigned-body.vhdl
ieee ieee/1993/std_logic_signed.vhdl
ieee ieee/1993/std_logic_signed-body.vhdl
ieee ieee/1993/std_logic_misc.vhdl
ieee ieee/1993/std_logic_misc-body.vhdl
ieee ieee/1993/vital_timing.vhdl
ieee ieee/1993/vital_timing-body.vhdl
ieee ieee/1993/vital_primitives.vhdl
ieee ieee/1993/vital_primitives-body.vhdl
ieee ieee/1993/vital_memory.vhdl
ieee ieee/1993/vital_memory-body.vhdl
```

## VHDL-2008

```order-2008
std std/2008/standard.vhdl
std std/2008/textio.vhdl
std std/2008/env.vhdl
ieee ieee/2008/std_logic_1164.vhdl
ieee ieee/2008/std_logic_1164-body.vhdl
ieee ieee/2008/std_logic_textio.vhdl
ieee ieee/2008/numeric_std.vhdl
ieee ieee/2008/numeric_std-body.vhdl
ieee ieee/2008/numeric_std_unsigned.vhdl
ieee ieee/2008/numeric_std_unsigned-body.vhdl
ieee ieee/2008/numeric_bit.vhdl
ieee ieee/2008/numeric_bit-body.vhdl
ieee ieee/2008/numeric_bit_unsigned.vhdl
ieee ieee/2008/numeric_bit_unsigned-body.vhdl
ieee ieee/2008/math_real.vhdl
ieee ieee/2008/math_real-body.vhdl
ieee ieee/2008/math_complex.vhdl
ieee ieee/2008/math_complex-body.vhdl
ieee ieee/2008/fixed_float_types.vhdl
ieee ieee/2008/fixed_generic_pkg.vhdl
ieee ieee/2008/fixed_generic_pkg-body.vhdl
ieee ieee/2008/fixed_pkg.vhdl
ieee ieee/2008/float_generic_pkg.vhdl
ieee ieee/2008/float_generic_pkg-body.vhdl
ieee ieee/2008/float_pkg.vhdl
ieee ieee/2008/std_logic_arith.vhdl
ieee ieee/2008/std_logic_arith-body.vhdl
ieee ieee/2008/std_logic_unsigned.vhdl
ieee ieee/2008/std_logic_unsigned-body.vhdl
ieee ieee/2008/std_logic_signed.vhdl
ieee ieee/2008/std_logic_signed-body.vhdl
ieee ieee/2008/std_logic_misc.vhdl
ieee ieee/2008/std_logic_misc-body.vhdl
ieee ieee/2008/vital_timing.vhdl
ieee ieee/2008/vital_timing-body.vhdl
ieee ieee/2008/vital_primitives.vhdl
ieee ieee/2008/vital_primitives-body.vhdl
ieee ieee/2008/vital_memory.vhdl
ieee ieee/2008/vital_memory-body.vhdl
ieee ieee/2008/ieee_std_context.vhdl
ieee ieee/2008/ieee_bit_context.vhdl
```

## VHDL-2019

```order-2019
std std/2019/standard.vhdl
std std/2019/textio.vhdl
std std/2019/env.vhdl
std std/2019/reflection.vhdl
ieee ieee/2019/std_logic_1164.vhdl
ieee ieee/2019/std_logic_1164-body.vhdl
ieee ieee/2019/std_logic_textio.vhdl
ieee ieee/2019/numeric_std.vhdl
ieee ieee/2019/numeric_std-body.vhdl
ieee ieee/2019/numeric_std_unsigned.vhdl
ieee ieee/2019/numeric_std_unsigned-body.vhdl
ieee ieee/2019/numeric_bit.vhdl
ieee ieee/2019/numeric_bit-body.vhdl
ieee ieee/2019/numeric_bit_unsigned.vhdl
ieee ieee/2019/numeric_bit_unsigned-body.vhdl
ieee ieee/2019/math_real.vhdl
ieee ieee/2019/math_real-body.vhdl
ieee ieee/2019/math_complex.vhdl
ieee ieee/2019/math_complex-body.vhdl
ieee ieee/2019/fixed_float_types.vhdl
ieee ieee/2019/fixed_generic_pkg.vhdl
ieee ieee/2019/fixed_generic_pkg-body.vhdl
ieee ieee/2019/fixed_pkg.vhdl
ieee ieee/2019/float_generic_pkg.vhdl
ieee ieee/2019/float_generic_pkg-body.vhdl
ieee ieee/2019/float_pkg.vhdl
ieee ieee/2019/std_logic_arith.vhdl
ieee ieee/2019/std_logic_arith-body.vhdl
ieee ieee/2019/std_logic_unsigned.vhdl
ieee ieee/2019/std_logic_unsigned-body.vhdl
ieee ieee/2019/std_logic_signed.vhdl
ieee ieee/2019/std_logic_signed-body.vhdl
ieee ieee/2019/std_logic_misc.vhdl
ieee ieee/2019/std_logic_misc-body.vhdl
ieee ieee/2019/vital_timing.vhdl
ieee ieee/2019/vital_timing-body.vhdl
ieee ieee/2019/vital_primitives.vhdl
ieee ieee/2019/vital_primitives-body.vhdl
ieee ieee/2019/vital_memory.vhdl
ieee ieee/2019/vital_memory-body.vhdl
ieee ieee/2019/ieee_std_context.vhdl
ieee ieee/2019/ieee_bit_context.vhdl
```
