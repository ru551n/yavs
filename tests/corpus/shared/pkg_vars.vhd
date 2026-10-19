-- difftest: top=pkg_vars relaxed
-- VHDL-93 style shared variables of non-protected types in a package (accepted with the
-- relaxed switch): one object each, read and written by several processes directly, through
-- package-body subprograms and through aliases. Processes that write in the same cycle only
-- add, and report a cycle later, so the result does not depend on process order.
library ieee;
use ieee.std_logic_1164.all;
package pkg_vars_p is
  type pair_t is record
    a : integer;
    b : boolean;
  end record;
  type pairs_t is array (1 to 3) of pair_t;
  type config_t is record
    name  : string(1 to 4);
    count : natural;
    bits  : std_logic_vector(7 downto 0);
    pairs : pairs_t;
  end record;
  shared variable count : integer := 10;
  shared variable flag : boolean;
  shared variable t : time := 5 ns;
  shared variable bits : std_logic_vector(3 downto 0) := "0101";
  shared variable text : string(1 to 5) := "hello";
  shared variable nums : integer_vector(0 to 3) := (others => 1);
  shared variable cfg : config_t := ("none", 0, x"0F", (others => (0, false)));
  procedure bump(n : integer);
  impure function total return integer;
  procedure set_pair(i : integer; v : integer);
end package;

package body pkg_vars_p is
  procedure bump(n : integer) is
  begin
    count := count + n;
    cfg.count := cfg.count + 1;
  end procedure;

  impure function total return integer is
    variable s : integer := 0;
  begin
    for i in nums'range loop
      s := s + nums(i);
    end loop;
    return s + count;
  end function;

  procedure set_pair(i : integer; v : integer) is
  begin
    cfg.pairs(i).a := cfg.pairs(i).a + v;
    cfg.pairs(i).b := true;
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.pkg_vars_p.all;
entity pkg_vars is
end entity;

architecture a of pkg_vars is
  -- A subprogram of the architecture that touches a package shared variable.
  procedure mark(i : integer) is
  begin
    nums(i) := nums(i) + 10;
  end procedure;
begin
  writer1 : process
    alias c is cfg.name;
  begin
    bump(1);
    mark(0);
    set_pair(1, 5);
    wait for 1 ns;
    c := "abcd";
    bits(0) := '1';
    bits(3 downto 2) := "11";
    wait for 10 ns;
    report "writer1 done";
    wait;
  end process;

  writer2 : process
    alias n is nums;
    alias first : integer is nums(1);
  begin
    bump(2);
    mark(3);
    set_pair(1, 7);
    wait for 2 ns;
    first := 100;
    n(2) := n(2) + 1;
    text(1) := 'J';
    flag := true;
    t := t * 2;
    wait;
  end process;

  checker : process
  begin
    wait for 0 ns;
    report "after delta: count=" & integer'image(count) & " total=" & integer'image(total);
    assert count = 13 severity failure;
    assert cfg.count = 2 severity failure;
    assert cfg.pairs(1).a = 12 and cfg.pairs(1).b severity failure;
    assert not cfg.pairs(2).b severity failure;
    assert nums = (11, 1, 1, 11) severity failure;
    wait for 5 ns;
    report "name=" & cfg.name & " text=" & text & " bits=" & to_string(bits) & " t=" & time'image(t);
    assert cfg.name = "abcd" severity failure;
    assert bits = "1101" severity failure;
    assert text = "Jello" severity failure;
    assert flag severity failure;
    assert t = 10 ns severity failure;
    assert nums = (11, 100, 2, 11) severity failure;
    assert total = 13 + 124 severity failure;
    assert cfg.bits = x"0F" severity failure;
    report "pkg_vars ok";
    wait;
  end process;
end architecture;
