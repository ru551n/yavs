-- difftest: top=nested_unconstrained_const std=2008
-- Arrays of arrays of an unconstrained array element (#329): the descriptor of a folded
-- constant carries every level, read through r(i)(j) in subprograms, as a signal, a variable,
-- a generic, a record field, and through slices of the middle level.
library ieee;
use ieee.numeric_std.all;

package nuc_pkg is
  type uvec is array (natural range <>) of unsigned;
  type uvecvec is array (natural range <>) of uvec;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.nuc_pkg.all;

entity nested_unconstrained_const is
  generic (gc : uvecvec := (0 => (x"1", x"6"), 1 => (x"7", x"8")));
end entity;

architecture a of nested_unconstrained_const is
  type ivec is array (natural range <>) of integer_vector;
  type ivecvec is array (natural range <>) of ivec;
  type rec is record
    n : integer;
    v : uvecvec(0 to 1)(0 to 1)(3 downto 0);
  end record;

  constant c  : uvecvec := (0 => (x"0", x"3"), 1 => (x"4", x"5"));
  constant ci : ivecvec := (0 => (0 => (1, 2), 1 => (3, 4)));
  constant cr : rec := (n => 1, v => (others => (x"1", x"2")));
  constant pc : uvecvec := (0 => (x"2", x"9"));
  signal s : uvecvec(0 to 1)(0 to 1)(3 downto 0) := (others => (x"6", x"7"));

  function chk(v : unsigned; r : uvecvec) return boolean is
  begin
    for i in r'range loop
      if v >= r(i)(0) and v <= r(i)(1) then return true; end if;
    end loop;
    return false;
  end function;

  function el(r : uvecvec; i, j : natural) return integer is
  begin
    return to_integer(r(i)(j));
  end function;

  function sl(r : uvecvec) return integer is
    variable m : uvec(0 to 0)(3 downto 0) := r(1)(0 to 0);
  begin
    return to_integer(m(0));
  end function;

  function eli(r : ivecvec; i, j, k : natural) return integer is
  begin
    return r(i)(j)(k);
  end function;
begin
  process
    variable u : unsigned(3 downto 0) := x"2";
    variable v : uvecvec(0 to 1)(0 to 1)(3 downto 0) := (others => (x"8", x"9"));
  begin
    assert chk(u, (0 => (x"0", x"3")));
    assert not chk(x"9", (0 => (x"0", x"3")));
    assert chk(x"5", c);
    assert el(c, 1, 1) = 5;
    assert el(c, 0, 1) = 3;
    assert c(1)(0) = x"4";
    assert c(0)(1)(0) = '0';
    assert sl(c) = 4;
    assert el(gc, 1, 1) = 8;
    assert gc(0)(1) = x"6";
    assert pc(0)(1) = x"9";
    assert el(cr.v, 1, 1) = 2;
    assert el(cr.v, 0, 0) = 1;
    assert eli(ci, 0, 1, 0) = 3;
    assert ci(0)(1)(1) = 4;
    assert el(s, 1, 1) = 7;
    assert el(v, 1, 0) = 8;
    v(1)(1) := x"A";
    assert el(v, 1, 1) = 10;
    assert sl(v) = 8;
    assert c(1 to 1)(1)(1) = x"5";
    assert el(c(1 to 1), 1, 1) = 5;
    report "done";
    wait;
  end process;
end architecture;
