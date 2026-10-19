-- Waveform test design for crates/yavs/tests/wave.rs: composite, shared and generic-sized
-- variables (`--wave-variables`). Analyzed and run with `--relaxed` (non-protected shared
-- variables).
package wv_pkg is
  shared variable pcount : integer := 0;
  type cell_t is protected
    procedure bump;
  end protected;
  impure function bump_width return natural;
end package;

package body wv_pkg is
  type cell_t is protected body
    variable n : integer := 0;
    procedure bump is
    begin
      n := n + 1;
    end procedure;
  end protected body;
  impure function bump_width return natural is
  begin
    pcount := pcount + 2;
    return pcount;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use std.textio.all;
use work.wv_pkg.all;

entity wv_sub is
  generic (DEPTH : positive := 4);
end entity;

architecture rtl of wv_sub is
  type pair_t is record
    a : std_logic;
    n : integer range 0 to 7;
  end record;
  type ints_t is array (natural range <>) of integer;
  type rec2_t is record
    v   : std_logic_vector(3 downto 0);
    b   : boolean;
    r   : real;
    arr : bit_vector(1 to 3);
    t   : time;
  end record;
  type slv_arr_t is array (0 to 1) of std_logic_vector(2 downto 0);
  type bool_arr_t is array (1 to 3) of boolean;
  shared variable acc : integer := 0;
  shared variable tab : ints_t(1 to 30) := (others => 0);
  shared variable cell : cell_t;
begin
  main : process
    variable cnt  : integer range 0 to DEPTH - 1 := 0;
    variable vec  : std_logic_vector(DEPTH / 4 - 1 downto 0) := (others => '0');
    variable bv   : bit_vector(0 to 2) := "001";
    variable pr   : pair_t := ('U', 0);
    variable ints : ints_t(1 to 2) := (others => 0);
    variable big  : ints_t(1 to 30) := (others => 0);
    variable sa   : slv_arr_t := ("000", "111");
    variable ba   : bool_arr_t := (others => false);
    variable r2   : rec2_t := ("0000", false, 0.5, "000", 0 ns);
    variable s    : string(1 to 2) := "ab";
    variable l    : line;
    variable dyn  : bit_vector(bump_width - 1 downto 0);
  begin
    for k in 1 to 3 loop
      cnt := cnt + 1;
      vec := not vec;
      bv := bv(1 to 2) & bv(0);
      pr := ('1', k);
      ints(k mod 2 + 1) := k * 10;
      s(1) := character'val(character'pos('a') + k);
      acc := acc + k;
      -- Written in some steps only; the last write stores the value already there.
      if k = 2 then
        big(7) := 77;
      elsif k = 3 then
        big(7) := 77;
        tab(3) := 6;
      elsif k = 1 then
        tab(3) := 5;
        sa(0) := "101";
        ba(2) := true;
        r2 := ("1010", true, 1.5, "101", 5 ns);
      end if;
      if k = 3 then
        sa(1) := "010";
        ba(2) := false;
        r2.arr := "110";
      end if;
      pcount := pcount + 1;
      cell.bump;
      wait for 10 ns;
    end loop;
    wait;
  end process;
end architecture;

entity wave_vars_tb is
end entity;

architecture sim of wave_vars_tb is
begin
  u : entity work.wv_sub generic map (DEPTH => 16);
end architecture;
