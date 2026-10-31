-- lint_780 and lint_781: what the source says outright cannot be evaluated.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity evaluated is
  generic (g : natural := 4);
end entity;

architecture rtl of evaluated is
  signal x : bit_vector(7 downto 0);
  signal up : bit_vector(3 to 7);
  signal by_g : bit_vector(g - 1 downto 0);
  signal i : integer := 9;
  constant zero : integer := 0;
  constant off : boolean := false;
  type matrix is array (0 to 1, 0 to 1) of bit;
  signal m : matrix;
  type rec is record
    f : bit_vector(1 downto 0);
  end record;
  signal r : rec;
  type vectors is array (0 to 1) of bit_vector(1 downto 0);
  signal vs : vectors;
  function f (n : integer) return bit is
  begin
    return '0';
  end function;
  signal uu : unsigned(7 downto 0);
begin
  assert (x(8)) = '1'; -- expect: lint_780
  assert (up(2)) = '1'; -- expect: lint_780
  assert (x(7) xor x(0) xor x(3)) = '1';
  assert (x(i)) = '1';
  assert (by_g(8)) = '1';
  assert (f(8)) = '1';
  assert (m(2, 0)) = '1'; -- expect: lint_780
  assert (r.f(2)) = '1'; -- expect: lint_780
  assert (vs(1)(2)) = '1'; -- expect: lint_780

  -- A constant turns this code off, so it is not judged.
  turned_off : if off generate
    assert (x(8)) = '1';
  end generate;

  p : process is
    variable n : integer := 7;
    variable d : real := 1.0;
    variable t : time := 1 ns;
    variable u : unsigned(7 downto 0) := (others => '0');
  begin
    n := n / 0; -- expect: lint_781
    n := n mod 0; -- expect: lint_781
    n := n rem 0; -- expect: lint_781
    d := d / 0.0; -- expect: lint_781
    n := t / 0 ns; -- expect: lint_781
    n := n / zero; -- expect: lint_781
    n := n / 2;
    n := n mod 8;
    n := n + 0;
    if n /= 0 then
      n := 1;
    end if;
    -- An overloaded operator defines its own behaviour.
    u := u / 0;
    if off then
      n := n / 0;
    elsif not off then
      n := 1;
    else
      n := n / 0;
    end if;
    wait;
  end process;
end architecture;
