-- difftest: top=tb_port_init_read std=2008
-- The initial value of a signal may read an `in` port, which holds the initial value of its
-- actual (also through two levels of ports), or a signal declared before it (#559).
library ieee;
use ieee.std_logic_1164.all;

package pk_pir is
  type pair is record
    a : std_logic;
    n : integer;
  end record;
end package;

library ieee;
use ieee.std_logic_1164.all;
use work.pk_pir.all;

entity leaf_pir is
  port (b : in std_logic; v : in std_logic_vector(3 downto 0); n : in integer; r : in pair;
        okb : out boolean);
end entity;

architecture rtl of leaf_pir is
  signal reg  : std_logic := b;
  signal vreg : std_logic_vector(3 downto 0) := v;
  signal sl   : std_logic_vector(1 downto 0) := v(2 downto 1);
  signal nreg : integer := n + 1;
  signal rreg : pair := r;
  signal loc  : std_logic := '1';
  signal loc2 : std_logic := not loc;
begin
  process begin
    okb <= reg = '1' and vreg = "1010" and sl = "01" and nreg = 8 and rreg.a = '0'
           and rreg.n = 3 and loc2 = '0';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.pk_pir.all;

entity mid_pir is
  port (b : in std_logic; v : in std_logic_vector(3 downto 0); n : in integer; r : in pair;
        okb : out boolean);
end entity;

architecture rtl of mid_pir is
begin
  u : entity work.leaf_pir port map (b => b, v => v, n => n, r => r, okb => okb);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.pk_pir.all;

entity tb_port_init_read is end entity;

architecture test of tb_port_init_read is
  signal s  : std_logic := '1';
  signal sv : std_logic_vector(3 downto 0) := "1010";
  signal sn : integer := 7;
  signal sr : pair := ('0', 3);
  signal ok : boolean;
begin
  u : entity work.mid_pir port map (b => s, v => sv, n => sn, r => sr, okb => ok);

  process begin
    wait for 1 ns;
    assert ok report "initial values read through ports differ" severity failure;
    report "ok";
    wait;
  end process;
end architecture;
