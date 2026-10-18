-- difftest: top=private_variables std=2019
-- 2019 private variables of a protected type (5.6.2): a scalar and a protected one,
-- used from the body; a method alias forwards a method of the private variable.
package private_variables_pkg is
  type counter_t is protected
    procedure inc (n : integer);
    impure function get return integer;
  end protected;
  type stats_t is protected
    private variable hits : counter_t;
    private variable last : integer := -1;
    procedure record_hit (v : integer);
    impure function last_value return integer;
    impure function doubled return integer;
    alias hit_count is hits.get [return integer];
  end protected;
end package;
package body private_variables_pkg is
  type counter_t is protected body
    variable c : integer := 0;
    procedure inc (n : integer) is begin c := c + n; end procedure;
    impure function get return integer is begin return c; end function;
  end protected body;
  type stats_t is protected body
    procedure record_hit (v : integer) is begin hits.inc(1); last := v; end procedure;
    impure function last_value return integer is begin return last; end function;
    -- The alias called with the implicit object forwards to this object's `hits`.
    impure function doubled return integer is begin return 2 * hit_count; end function;
  end protected body;
end package body;
use work.private_variables_pkg.all;
entity private_variables is end entity;
architecture t of private_variables is
  shared variable s, other : stats_t;
begin
  process begin
    report "last=" & integer'image(s.last_value) & " hits=" & integer'image(s.hit_count);
    s.record_hit(7);
    s.record_hit(9);
    report "last=" & integer'image(s.last_value) & " hits=" & integer'image(s.hit_count);
    other.record_hit(1);
    report "doubled=" & integer'image(s.doubled) & " other=" & integer'image(other.doubled);
    wait;
  end process;
end architecture;
