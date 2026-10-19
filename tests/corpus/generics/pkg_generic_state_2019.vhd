-- difftest: top=pkg_generic_state_2019 std=2019
-- Generic packages with signals, shared variables of protected types and body
-- state: every instance has its own objects (VHDL-2019 copy).
package state_g is
  generic (init : integer; width : positive);
  type counter_t is protected
    procedure bump(by : integer);
    impure function get return integer;
  end protected;
  signal s : integer := init;
  signal v : bit_vector(width - 1 downto 0) := (others => '0');
  constant ones : bit_vector(width - 1 downto 0) := (others => '1');
  procedure log(x : integer);
  impure function calls return integer;
  procedure bump(by : integer);
  impure function cnt_get return integer;
end package;
package body state_g is
  type counter_t is protected body
    variable n : integer := init;
    procedure bump(by : integer) is
    begin
      n := n + by;
    end procedure;
    impure function get return integer is
    begin
      return n;
    end function;
  end protected body;
  shared variable count : counter_t;
  shared variable cnt : counter_t;
  procedure bump(by : integer) is
  begin
    cnt.bump(by);
  end procedure;
  impure function cnt_get return integer is
  begin
    return cnt.get;
  end function;
  procedure log(x : integer) is
  begin
    count.bump(1);
    report "log " & integer'image(init) & ": " & integer'image(x);
  end procedure;
  impure function calls return integer is
  begin
    return count.get - init;
  end function;
end package body;

package st_a is new work.state_g generic map (init => 10, width => 4);
package st_b is new work.state_g generic map (init => 20, width => 8);

entity pkg_generic_state_2019 is end entity;
architecture t of pkg_generic_state_2019 is
begin
  driver_a : process
  begin
    work.st_a.s <= 11;
    work.st_a.v <= work.st_a.ones;
    wait for 1 ns;
    work.st_a.s <= 12;
    wait;
  end process;

  driver_b : process
  begin
    wait for 2 ns;
    work.st_b.s <= 21;
    work.st_b.v <= x"a5";
    wait;
  end process;

  watcher : process (work.st_a.s, work.st_b.s)
  begin
    report "watch a=" & integer'image(work.st_a.s) & " b=" & integer'image(work.st_b.s);
  end process;

  main : process
  begin
    work.st_a.bump(1);
    work.st_a.bump(2);
    work.st_b.bump(5);
    work.st_a.log(1);
    work.st_a.log(2);
    work.st_b.log(3);
    wait for 5 ns;
    report "cnt a=" & integer'image(work.st_a.cnt_get) & " b=" & integer'image(work.st_b.cnt_get);
    report "calls a=" & integer'image(work.st_a.calls) & " b=" & integer'image(work.st_b.calls);
    report "v a=" & to_string(work.st_a.v) & " b=" & to_string(work.st_b.v);
    report "len a=" & integer'image(work.st_a.v'length) & " b=" & integer'image(work.st_b.v'length);
    assert work.st_b.s = 21 severity failure;
    wait;
  end process;
end architecture;
