-- The analysis cache of cycle mode: top-level generics each read by another part of the work.
-- `W` sizes the DUT (the elaboration walk), `L` only through `'length` of a constant (the walk),
-- `F` only in a globally static pure call of a member (the value passes), `K` only in a member's
-- body (the selection), `TAG` only by the testbench at run time, through a procedure (nothing
-- before the run).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity acache_dut is
  generic (W : positive; N : natural);
  port (
    clk : in std_ulogic;
    cnt : out unsigned(W - 1 downto 0)
  );
end entity;

architecture rtl of acache_dut is
  signal c : unsigned(W - 1 downto 0) := (others => '0');
begin
  count : process (clk)
  begin
    if rising_edge(clk) then
      c <= c + 1 + N;
    end if;
  end process;
  cnt <= c;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity acache is
  generic (
    W : positive := 8;
    L : string := "ab";
    F : natural := 2;
    K : natural := 3;
    TAG : string := "x"
  );
end entity;

architecture tb of acache is
  function twice(x : natural) return natural is
  begin
    return 2 * x;
  end function;
  -- `TAG` reaches the report through a procedure, as a test runner's configuration does.
  procedure show(t : string; c, a, b : natural) is
  begin
    report t & " cnt=" & integer'image(c) & " a=" & integer'image(a) & " b=" & integer'image(b);
  end procedure;
  constant LL : natural := L'length;
  signal clk : std_ulogic := '0';
  signal cnt : unsigned(W - 1 downto 0);
  signal a, b : natural range 0 to 999 := 0;
begin
  dut : entity work.acache_dut generic map (W => W, N => LL) port map (clk, cnt);
  -- Members of the top-level unit: `K` in a body, `F` in a folded call.
  regk : process (clk)
  begin
    if rising_edge(clk) then
      a <= (a + K) mod 1000;
    end if;
  end process;
  regf : process (clk)
  begin
    if rising_edge(clk) then
      b <= (b + twice(F)) mod 1000;
    end if;
  end process;
  main : process
  begin
    for i in 1 to 20 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    show(TAG, to_integer(cnt), a, b);
    wait;
  end process;
end architecture;
