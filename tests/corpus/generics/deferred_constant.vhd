-- difftest: top=tb_deferred_constant
-- A deferred constant (LRM 4.8) used during elaboration: in a generic default, a
-- component generic default, a signal range and a generate bound. The value comes from
-- the full declaration in the package body.
package defaults is
  constant TPERIOD : time;
  constant BRANCHES : natural;
  constant WIDTH : integer;
end package;

package body defaults is
  function twice(x : natural) return natural is
  begin
    return 2 * x;
  end function;
  constant TPERIOD : time := 10 ns;
  constant BRANCHES : natural := twice(3);
  constant WIDTH : integer := BRANCHES + 1;
end package body;

use work.defaults.all;
entity leaf is
  generic (tperiod_clk : time := TPERIOD; n : natural := BRANCHES);
end entity;

architecture a of leaf is
  signal v : bit_vector(WIDTH - 1 downto 0) := (others => '1');
begin
  process begin
    assert tperiod_clk = 10 ns report "bad period" severity failure;
    assert n = 6 report "bad n" severity failure;
    assert v'length = 7 report "bad width" severity failure;
    report "period " & time'image(tperiod_clk) & " n " & integer'image(n);
    wait;
  end process;
  g : for i in 0 to BRANCHES - 1 generate
    process begin
      wait for 1 ns * i;
      report "branch " & integer'image(i);
      wait;
    end process;
  end generate;
end architecture;

entity tb_deferred_constant is end entity;

architecture t of tb_deferred_constant is
  use work.defaults.all;
  component leaf is
    generic (tperiod_clk : time := TPERIOD; n : natural := BRANCHES);
  end component;
begin
  u : leaf;
end architecture;
