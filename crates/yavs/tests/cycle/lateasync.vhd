-- Cycle mode: a register with an asynchronous reset outside the region, on a copy of the clock
-- (its edges a delta later), samples combinational logic fed by a member register on the clock.
-- It stays out for a `while` loop the lowering does not take, though the checker fits it, so it
-- was exempt from the delta seeds; the logic stayed in the region and the register sampled it
-- settled, a delta before the normal mode's new value. Found by the fuzzer.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_lateasync is
  generic (W : positive := 8; G : boolean := true);
end entity;
architecture a of cycle_lateasync is
  signal clk, dk, arst : std_ulogic := '0';
  signal cnt, c, q : unsigned(W - 1 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns;
  dk <= clk when G else '0';
  first : process (clk)
  begin
    if rising_edge(clk) then
      cnt <= cnt + 3;
    end if;
  end process;
  c <= cnt xor shift_right(cnt, 1);
  late : process (dk, arst)
    variable k : integer range 0 to 4;
  begin
    if arst = '1' then
      q <= (others => '0');
    elsif dk'event and dk = '1' then
      k := 0;
      while k < 2 loop
        k := k + 1;
      end loop;
      q <= c;
    end if;
  end process;
  stim : process
  begin
    for i in 0 to 39 loop
      wait until falling_edge(clk);
      report "cnt=" & to_hstring(cnt) & " q=" & to_hstring(q);
    end loop;
    std.env.finish;
  end process;
end architecture;
