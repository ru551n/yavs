-- `--two-state` (A53, A64, A65): below the top-level unit every logic value is 0 or 1. What the
-- unit reads from the testbench maps as the cycle boundary does ('1' and 'H' to 1, anything else
-- to 0), its metavalue literals take --init-x ('L' is 0, 'H' is 1), what it declares without an
-- initial value starts at --init-unset (0 without it). The testbench keeps the nine values.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package twostate_pkg is
  type rec is record
    a : std_logic;
    v : std_logic_vector(3 downto 0);
  end record;
  procedure getx(variable x : out std_logic_vector);
  function fx return std_logic;
end package;

package body twostate_pkg is
  procedure getx(variable x : out std_logic_vector) is
  begin
    x := (x'range => 'X');
  end procedure;
  function fx return std_logic is
  begin
    return 'W';
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.twostate_pkg.all;

entity twostate_dut is
  generic (W : positive);
  port (
    clk : in std_logic;
    d   : in std_logic_vector(W - 1 downto 0);
    a   : in std_logic;
    rin : in rec;
    q   : out std_logic_vector(W - 1 downto 0);
    nq  : out std_logic_vector(W - 1 downto 0);
    z   : out std_logic_vector(W - 1 downto 0);
    lit : out std_logic_vector(W - 1 downto 0);
    flags : out std_logic_vector(7 downto 0);
    rout : out rec;
    cnt : out std_logic_vector(3 downto 0)
  );
end entity;

architecture rtl of twostate_dut is
  signal u : std_logic_vector(W - 1 downto 0);
begin
  nq <= not d;
  z <= u;
  process (clk)
    variable n : unsigned(3 downto 0);
    variable g : std_logic_vector(W - 1 downto 0);
  begin
    if rising_edge(clk) then
      q <= d;
      rout <= rin;
      n := n + 1;
      cnt <= std_logic_vector(n);
      lit <= (others => 'X');
      getx(g);
      flags(0) <= g(0) or g(W - 1);
      flags(1) <= fx;
      flags(2) <= 'H';
      flags(3) <= 'L';
      if a = 'X' then
        flags(4) <= '1';
      else
        flags(4) <= '0';
      end if;
      if std_match(d(0 downto 0), "-") then
        flags(5) <= '1';
      else
        flags(5) <= '0';
      end if;
      flags(6) <= a;
      flags(7) <= 'Z';
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.twostate_pkg.all;

entity twostate is
end entity;

architecture sim of twostate is
  type widths is array (natural range <>) of positive;
  constant WS : widths := (1, 63, 64, 65, 128);
  signal clk : std_logic := '0';
  signal a : std_logic := 'X';
  signal rin : rec := ('X', "1HZL");
begin
  g : for i in WS'range generate
    signal d, q, nq, z, lit : std_logic_vector(WS(i) - 1 downto 0);
    signal flags : std_logic_vector(7 downto 0);
    signal rout : rec;
    signal cnt : std_logic_vector(3 downto 0);
  begin
    -- 'H' at the top and bit 0 '1', the rest metavalues.
    process
      variable v : std_logic_vector(WS(i) - 1 downto 0);
    begin
      v := (others => 'U');
      v(0) := '1';
      v(v'left) := 'H';
      d <= v;
      wait;
    end process;
    u : entity work.twostate_dut
      generic map (W => WS(i))
      port map (clk, d, a, rin, q, nq, z, lit, flags, rout, cnt);
    process
    begin
      wait for 1 ns;
      clk <= '1';
      wait for 1 ns;
      report "W=" & integer'image(WS(i)) & " q=" & to_hstring(q) & " nq=" & to_hstring(nq)
        & " z=" & to_hstring(z) & " lit=" & to_hstring(lit) & " flags=" & to_string(flags)
        & " rout=" & to_string(rout.a) & to_string(rout.v) & " cnt=" & to_string(cnt)
        & " d=" & to_string(d(0));
      wait;
    end process;
  end generate;
end architecture;
