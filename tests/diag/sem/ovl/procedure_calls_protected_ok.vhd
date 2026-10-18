-- yavs-test: std=2008
-- expect: none
-- LRM 10.7, 5.6 (method calls through a shared variable)
package pp is
  type counter is protected
    procedure add (n : integer := 1);
    impure function get return integer;
  end protected;
end package;
package body pp is
  type counter is protected body
    variable v : integer := 0;
    procedure add (n : integer := 1) is
    begin
      v := v + n;
    end procedure;
    impure function get return integer is
    begin
      return v;
    end function;
  end protected body;
end package body;
use work.pp.all;
entity e is end entity;
architecture a of e is
  shared variable c : counter;
begin
  process
    variable n : integer;
  begin
    c.add;
    c.add(2);
    c.add(n => 3);
    n := c.get;
    wait;
  end process;
end architecture;
