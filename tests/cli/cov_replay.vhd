-- An elaboration-time call of a function that updates package state: under --cov the simulation
-- replays it, then calls the same function from a process (#705).
package reg_pkg is
  type reg_t is protected
    impure function new_id(name : string; parent : integer := 0; mode : integer := 1) return integer;
  end protected;
  impure function make_id(name : string; parent : integer := 0) return integer;
end package;

package body reg_pkg is
  type reg_t is protected body
    variable count : integer := 0;
    impure function new_id(name : string; parent : integer := 0; mode : integer := 1) return integer is
      variable low : string(name'range);
    begin
      for i in name'range loop
        low(i) := character'val(character'pos(name(i)) + 1);
      end loop;
      count := count + 1;
      if parent > count then
        return -1;
      end if;
      return count * 10 + low'length + mode;
    end function;
  end protected body;

  shared variable reg : reg_t;

  impure function make_id(name : string; parent : integer := 0) return integer is
  begin
    return reg.new_id(name, parent);
  end function;
end package body;

use work.reg_pkg.all;

entity cov_replay is
end entity;

architecture sim of cov_replay is
  constant first : integer := make_id("elab");
begin
  process
    variable a, b : integer;
  begin
    a := make_id("process");
    b := make_id("again", 1);
    assert first = 15 report "first " & integer'image(first) severity failure;
    assert a = 28 report "a " & integer'image(a) severity failure;
    assert b = 36 report "b " & integer'image(b) severity failure;
    report "ok";
    wait;
  end process;
end architecture;
