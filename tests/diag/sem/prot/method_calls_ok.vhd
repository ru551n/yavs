-- yavs-test: std=2008
-- expect: none
-- LRM 4.5.4, 5.6.2 (method calls through an object, overloads, parameterless functions, shared variables, calls inside the body)
package pk is
  type counter_t is protected
    procedure inc;
    procedure inc (n : integer);
    impure function get return integer;
    impure function get_plus (n : integer) return integer;
    impure function name return string;
  end protected;
end package;

package body pk is
  type counter_t is protected body
    variable v : integer := 0;
    procedure inc is begin v := v + 1; end procedure;
    procedure inc (n : integer) is begin v := v + n; inc; end procedure;
    impure function get return integer is begin return v; end function;
    impure function get_plus (n : integer) return integer is begin return get + n; end function;
    impure function name return string is begin return "c"; end function;
  end protected body;
end package body;

use work.pk.all;
package pk2 is
  shared variable sc : counter_t;
end package;

use work.pk.all;
use work.pk2.all;
entity e is end entity;
architecture a of e is
begin
  process
    variable c : counter_t;
    variable x : integer;
    variable ch : character;
  begin
    c.inc;
    c.inc(3);
    c.inc(n => 2);
    x := c.get + c.get_plus(1);
    ch := c.name(1);
    sc.inc;
    work.pk2.sc.inc(2);
    x := work.pk2.sc.get;
    report c.name;
    wait;
  end process;
end architecture;
