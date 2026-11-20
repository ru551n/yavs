-- LRM 14.3.2, 14.4.2.1: the subtype of a port is elaborated with the port clause, so an
-- impure function in an out port's range is called during elaboration, once (#1177).
package cnt is
  type counter is protected
    impure function bump (by : integer) return integer;
  end protected;
end package;
package body cnt is
  type counter is protected body
    variable v : integer := 0;
    impure function bump (by : integer) return integer is
    begin
      v := v + by;
      return v;
    end function;
  end protected body;
end package body;
use work.cnt.all;
package fn is
  shared variable c : counter;
  impure function counted (x : integer) return integer;
end package;
package body fn is
  impure function counted (x : integer) return integer is
    variable n : integer;
  begin
    n := c.bump(1);
    return x;
  end function;
end package body;
use work.fn.all;
entity sub is
  port (o : out integer range 0 to counted(21) := 0);
end entity;
architecture a of sub is
begin
  process begin
    wait for 1 ns;
    o <= 7;
    wait;
  end process;
end architecture;
use work.fn.all;
entity top is end entity;
architecture a of top is
  signal b : integer range 0 to 21 := 0;
begin
  u : entity work.sub port map (o => b);
  process begin
    report "RES calls=" & integer'image(c.bump(0));
    wait for 2 ns;
    report "RES b=" & integer'image(b) & " calls=" & integer'image(c.bump(0));
    wait;
  end process;
end architecture;
