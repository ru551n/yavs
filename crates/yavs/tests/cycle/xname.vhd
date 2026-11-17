-- Cycle mode: a testbench reads signals inside the design through external names: `dbl` through
-- an alias of the process, `cnt` named directly, `fast` through an alias of a block
-- in the actual of a port.
-- Only those are read outside, so `hid`, which only the design reads, is not exported.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_xname_dut is
  port (clk : in std_ulogic; q : out unsigned(7 downto 0));
end entity;
architecture a of cycle_xname_dut is
  signal cnt : unsigned(7 downto 0) := (others => '0');
  signal hid : unsigned(7 downto 0) := x"5A";
  signal dbl, fast : unsigned(7 downto 0);
begin
  reg : process (clk)
  begin
    if rising_edge(clk) then
      cnt <= cnt + 3;
      hid <= hid xor cnt;
    end if;
  end process;
  dbl <= cnt + hid;
  fast <= cnt + 1;
  q <= hid;
end architecture;

library ieee;
use ieee.numeric_std.all;
entity cycle_xname_watch is
  port (v : in unsigned(7 downto 0); changes : out natural; last : out unsigned(7 downto 0));
end entity;
architecture a of cycle_xname_watch is
begin
  watch : process
    variable n : natural := 0;
  begin
    wait on v;
    n := n + 1;
    changes <= n;
    last <= v;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_xname is
end entity;
architecture a of cycle_xname is
  signal clk : std_ulogic := '0';
  signal q : unsigned(7 downto 0);
  signal changes : natural;
  signal last : unsigned(7 downto 0);
begin
  dut : entity work.cycle_xname_dut port map (clk => clk, q => q);
  main : process
    alias dbl_a is <<signal .cycle_xname.dut.dbl : unsigned(7 downto 0)>>;
    variable sum : natural := 0;
  begin
    for i in 1 to 20 loop
      clk <= '1';
      wait for 5 ns;
      sum := sum + to_integer(dbl_a) + 3 * to_integer(<<signal .cycle_xname.dut.cnt : unsigned(7 downto 0)>>);
      clk <= '0';
      wait for 5 ns;
    end loop;
    report "sum=" & to_string(sum) & " q=" & to_hstring(q) & " fast=" & to_string(changes)
      & " last=" & to_hstring(last);
    wait;
  end process;
  -- After `dut`, so the external name is elaborated.
  b : block
    alias fast_b is <<signal .cycle_xname.dut.fast : unsigned(7 downto 0)>>;
  begin
    -- An expression as the actual: a process of its own reads the alias.
    watch : entity work.cycle_xname_watch
      port map (v => fast_b xor x"00", changes => changes, last => last);
  end block;
end architecture;

