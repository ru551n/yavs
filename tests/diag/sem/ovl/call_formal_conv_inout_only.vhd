-- yavs-test: std=2008
-- expect: E1313 23:20 inout formal conversion without an actual conversion
-- help: conversion for the way in
-- LRM 6.5.7.1
-- An `inout` parameter with a formal conversion also needs an actual conversion for the way in.
entity e is end entity;
architecture a of e is
  function to_r(i : integer) return real is begin return real(i); end function;
  function to_i(r : real) return integer is begin return integer(r); end function;
  procedure seti(variable x : out integer) is begin x := 3; end procedure;
  procedure inc(variable x : inout integer) is begin x := x + 1; end procedure;
  procedure geti(variable x : in integer) is begin end procedure;
  procedure sigi(signal x : in integer) is begin end procedure;
  procedure sigo(signal x : out integer) is begin x <= 1; end procedure;
  procedure setv(variable x : out bit_vector) is begin x := (x'range => '1'); end procedure;
  signal sr : real;
begin
  process
    variable r : real := 1.0;
    variable i : integer := 5;
    variable b : bit_vector(0 to 3);
  begin
    inc(to_r(x) => r);
    wait;
  end process;
end architecture;
