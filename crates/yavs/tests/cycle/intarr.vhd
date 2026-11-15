-- Arrays of integers and enumerations in cycle mode against the normal mode: variables written
-- at run-time indices with aggregate start values (integers and an enumeration), a variable of a
-- combinational process, and a constant table read at a run-time index. The processes that drive
-- signals of such arrays stay out, with the reason.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_intarr is
end entity;
architecture a of cycle_intarr is
  type nat_arr is array (natural range <>) of natural range 0 to 255;
  type int_arr is array (natural range <>) of integer;
  type state_t is (idle, busy, done_s, err);
  type state_arr is array (1 to 3) of state_t;
  constant tab : int_arr(0 to 7) := (3, -1, 4, 1, -5, 9, 2, 6);
  signal clk : std_ulogic := '0';
  signal x : unsigned(7 downto 0) := x"00";
  signal sh : nat_arr(0 to 3);
  signal sts : state_arr;
  signal acc0, acc3, pick, tsum, sel : integer := 0;
  signal stp : state_t := idle;
begin
  lanes : for l in 0 to 3 generate
    lane : process (x)
    begin
      sh(l) <= to_integer(x(l + 3 downto l)) * (l + 1);
    end process;
  end generate;

  st : process (all)
  begin
    for i in 1 to 3 loop
      if x(i) = '1' then
        sts(i) <= state_t'val(i);
      else
        sts(i) <= idle;
      end if;
    end loop;
  end process;

  reg : process (clk)
    variable v : int_arr(0 to 3) := (others => 7);
    variable sv : state_arr := (others => idle);
    variable k : natural range 0 to 3;
  begin
    if rising_edge(clk) then
      k := to_integer(x(1 downto 0));
      v(k) := v(k) + to_integer(x(5 downto 2)) - tab(to_integer(x(2 downto 0)));
      v(0) := v(0) + 1;
      case x(3 downto 2) is
        when "00" => sv(k mod 3 + 1) := idle;
        when "01" => sv(k mod 3 + 1) := busy;
        when "10" => sv(k mod 3 + 1) := done_s;
        when others => sv(k mod 3 + 1) := err;
      end case;
      acc0 <= v(0);
      acc3 <= v(3);
      pick <= v(k) + tab(k + 4);
      tsum <= v(0) + v(1) + v(2) + v(3);
      stp <= sv(to_integer(x(7 downto 6)) mod 3 + 1);
    end if;
  end process;

  cmb : process (all)
    variable cnt : nat_arr(0 to 3);
  begin
    for i in 0 to 3 loop
      cnt(i) := to_integer(x(i + 3 downto i)) * (i + 1);
    end loop;
    sel <= cnt(to_integer(x(1 downto 0))) + cnt(3);
  end process;

  main : process
    variable r : unsigned(15 downto 0) := x"ACE1";
  begin
    wait for 1 ns;
    for c in 1 to 16 loop
      r := r(14 downto 0) & (r(15) xor r(13) xor r(12) xor r(10));
      x <= r(7 downto 0);
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      report "c=" & integer'image(c) & " sh=" & integer'image(sh(0)) & integer'image(sh(3))
        & " sts=" & state_t'image(sts(1)) & state_t'image(sts(3)) & " acc="
        & integer'image(acc0) & integer'image(acc3) & " pick=" & integer'image(pick)
        & " sel=" & integer'image(sel)
        & " tsum=" & integer'image(tsum) & " stp=" & state_t'image(stp);
    end loop;
    wait;
  end process;
end architecture;
