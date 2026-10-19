-- difftest: top=variable_ports std=2019
-- 2019 variable ports (6.5.2, LCS-2016-047): instances share one protected object through
-- ports of class variable, associated with a shared variable, another variable port (through
-- a middle entity and a block), a package shared variable and a component port.
package variable_ports_pkg is
  type acc_t is protected
    procedure add (n : integer);
    impure function total return integer;
    impure function calls return integer;
  end protected;
end package;
package body variable_ports_pkg is
  type acc_t is protected body
    variable sum : integer := 0;
    variable n_calls : integer := 0;
    procedure add (n : integer) is
    begin
      sum := sum + n;
      n_calls := n_calls + 1;
    end procedure;
    impure function total return integer is
    begin
      return sum;
    end function;
    impure function calls return integer is
    begin
      return n_calls;
    end function;
  end protected body;
end package body;

use work.variable_ports_pkg.all;
package variable_ports_globals is
  shared variable global : acc_t;
end package;

use work.variable_ports_pkg.all;
entity vp_worker is
  generic (id : integer);
  port (variable acc : inout acc_t);
end entity;
architecture a of vp_worker is
begin
  -- Two processes of each instance add at distinct times; the checker reads after all.
  p1 : process
  begin
    wait for id * 1 ns;
    acc.add(id);
    wait;
  end process;
  p2 : process
  begin
    wait for id * 1 ns + 10 ns;
    acc.add(100 * id);
    report "worker " & integer'image(id) & " sees total " & integer'image(acc.total);
    wait;
  end process;
end architecture;

use work.variable_ports_pkg.all;
entity vp_middle is
  generic (id : integer);
  port (variable shared_acc : inout acc_t);
end entity;
architecture a of vp_middle is
begin
  u : entity work.vp_worker generic map (id => id) port map (acc => shared_acc);
  b : block
    port (variable inner : inout acc_t);
    port map (inner => shared_acc);
  begin
    w : entity work.vp_worker generic map (id => id + 1) port map (acc => inner);
  end block;
end architecture;

use work.variable_ports_pkg.all;
use work.variable_ports_globals.all;
entity variable_ports is
end entity;
architecture a of variable_ports is
  shared variable local : acc_t;
  component vp_worker is
    generic (id : integer);
    port (variable acc : inout acc_t);
  end component;
begin
  w1 : entity work.vp_worker generic map (id => 1) port map (acc => local);
  m : entity work.vp_middle generic map (id => 2) port map (shared_acc => local);
  g : for i in 4 to 5 generate
    w : entity work.vp_worker generic map (id => i) port map (acc => local);
  end generate;
  c : component vp_worker generic map (id => 6) port map (acc => global);
  wg : entity work.vp_worker generic map (id => 7) port map (acc => global);
  check : process
  begin
    local.add(1000);
    wait for 50 ns;
    report "local total " & integer'image(local.total) & " calls " & integer'image(local.calls);
    report "global total " & integer'image(global.total) & " calls " & integer'image(global.calls);
    assert local.total = 1000 + 101 * (1 + 2 + 3 + 4 + 5) severity failure;
    assert local.calls = 11 severity failure;
    assert global.total = 101 * (6 + 7) and global.calls = 4 severity failure;
    wait;
  end process;
end architecture;
