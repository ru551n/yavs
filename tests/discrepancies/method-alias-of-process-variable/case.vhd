-- LRM 2019 5.6.2, 6.6.3: an alias of a method of a process variable, called through the alias.
package p is
  type counter_t is protected
    procedure add (n : integer);
    impure function get return integer;
  end protected;
end package;
package body p is
  type counter_t is protected body
    variable c : integer := 0;
    procedure add (n : integer) is begin c := c + n; end procedure;
    impure function get return integer is begin return c; end function;
  end protected body;
end package body;
use work.p.all;
entity top is end entity;
architecture a of top is
begin
  process
    variable lv : counter_t;
    alias lv_add is lv.add [integer];
  begin
    lv_add(4);
    report "RES lv=" & integer'image(lv.get);
    wait;
  end process;
end architecture;
