-- difftest: top=variable_ports_same_delta std=2019
-- 2019 variable ports (6.5.2, LCS-2016-047): processes of several instances call methods of
-- one protected object in the same delta cycles; the top level has no process of its own and
-- the shared variables are declared in a generate. Method calls are atomic (LRM 5.6.2), so
-- the totals do not depend on the order the processes run in.
package variable_ports_same_delta_pkg is
  type tally_t is protected
    procedure add (v : integer);
    impure function total return integer;
    impure function count return integer;
  end protected;
end package;
package body variable_ports_same_delta_pkg is
  type tally_t is protected body
    variable sum : integer := 0;
    variable n : integer := 0;
    procedure add (v : integer) is
    begin
      sum := sum + v;
      n := n + 1;
    end procedure;
    impure function total return integer is
    begin
      return sum;
    end function;
    impure function count return integer is
    begin
      return n;
    end function;
  end protected body;
end package body;

use work.variable_ports_same_delta_pkg.all;
entity vpsd_adder is
  generic (id : integer);
  port (variable t : inout tally_t);
end entity;
architecture a of vpsd_adder is
begin
  gen : for k in 1 to 2 generate
    p : process
    begin
      for step in 1 to 20 loop
        t.add(id * k);
        wait for 1 ns;
      end loop;
      wait;
    end process;
  end generate;
end architecture;

use work.variable_ports_same_delta_pkg.all;
entity vpsd_checker is
  generic (gid : integer; expect_total : integer; expect_count : integer);
  port (variable t : inout tally_t);
end entity;
architecture a of vpsd_checker is
begin
  process
  begin
    wait for 9500 ps + gid * 1 fs;
    report "group " & integer'image(gid) & " count before 10 ns " & integer'image(t.count);
    wait for 100 ns + gid * 1 ns;
    report "group " & integer'image(gid) & " total " & integer'image(t.total)
      & " count " & integer'image(t.count);
    assert t.total = expect_total and t.count = expect_count severity failure;
    wait;
  end process;
end architecture;

entity variable_ports_same_delta is
end entity;
architecture a of variable_ports_same_delta is
begin
  g : for grp in 0 to 1 generate
    shared variable tally : work.variable_ports_same_delta_pkg.tally_t;
  begin
    w : for i in 1 to 4 generate
      u : entity work.vpsd_adder generic map (id => 10 * grp + i) port map (t => tally);
    end generate;
    c : entity work.vpsd_checker
      generic map (gid => grp, expect_total => 20 * 3 * (40 * grp + 10), expect_count => 160)
      port map (t => tally);
  end generate;
end architecture;
