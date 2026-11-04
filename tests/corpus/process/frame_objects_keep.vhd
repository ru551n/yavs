-- difftest: top=frame_objects_keep
-- Composite variables of a process live in its frame: they keep their values across suspensions
-- and in procedures that wait, by-reference calls reach them, neighbours stay untouched.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity frame_objects_keep is
end entity;

architecture a of frame_objects_keep is
  type small is record
    a : integer;
    b : std_ulogic_vector(7 downto 0);
    c : boolean;
  end record;
  type regs is array (0 to 15) of std_ulogic_vector(31 downto 0);
  type big is record
    s    : small;
    r    : regs;
    n    : natural;
    tail : std_ulogic_vector(15 downto 0);
  end record;
  type smalls is array (0 to 3) of small;
  type uc is record
    x : bit_vector;
    y : integer;
  end record;
  type small_ptr is access small;

  type counter_t is protected
    procedure add (x : inout small);
    impure function total return integer;
  end protected;
  type counter_t is protected body
    variable t : integer := 0;
    procedure add (x : inout small) is
    begin
      t := t + x.a;
      x.a := x.a + 1;
    end procedure;
    impure function total return integer is
    begin
      return t;
    end function;
  end protected body;
  shared variable counter : counter_t;

  signal clk : std_ulogic := '0';
  signal sr  : big := ((0, x"00", false), (others => (others => '0')), 0, x"0000");
  signal done : boolean := false;

  function sum (v : big) return integer is
    variable t : integer := v.s.a + v.n;
  begin
    for i in v.r'range loop
      t := (t + to_integer(unsigned(v.r(i)(7 downto 0)))) mod 65536;
    end loop;
    return t;
  end function;

  function field_of (v : small) return std_ulogic_vector is
  begin
    return v.b;
  end function;

  procedure bump (x : inout small) is
  begin
    x.a := x.a + 3;
    x.b := std_ulogic_vector(unsigned(x.b) + 1);
    x.c := not x.c;
  end procedure;

  procedure fill (x : out big; k : integer) is
  begin
    x.s := (k, std_ulogic_vector(to_unsigned(k mod 256, 8)), k mod 2 = 0);
    for i in x.r'range loop
      x.r(i) := std_ulogic_vector(to_unsigned(k * i + 1, 32));
    end loop;
    x.n := k;
    x.tail := x"beef";
  end procedure;

  procedure waits (x : inout small; signal c : in std_ulogic; n : integer) is
  begin
    for i in 1 to n loop
      wait until rising_edge(c);
      bump(x);
    end loop;
  end procedure;

  procedure deep3 (x : inout small) is
  begin
    bump(x);
  end procedure;
  procedure deep2 (x : inout small) is
  begin
    deep3(x);
    deep3(x);
  end procedure;

  procedure uc_set (x : out uc) is
  begin
    for i in x.x'range loop
      x.x(i) := '1';
    end loop;
    x.y := x.x'length;
  end procedure;
begin
  clk <= not clk after 5 ns when not done;

  -- A record driven whole, past the size of a small temporary, and read back whole.
  driver : process (clk)
    variable v : big := ((1, x"01", true), (others => x"0000000f"), 2, x"1234");
  begin
    if rising_edge(clk) then
      v.s.a := v.s.a + 1;
      v.r(v.s.a mod 16) := std_ulogic_vector(unsigned(v.r(v.s.a mod 16)) + 3);
      sr <= v;
    end if;
  end process;

  main : process
    variable before : regs := (others => x"aaaaaaaa");
    variable v      : big;
    variable behind : regs := (others => x"55555555");
    variable sm     : small := (10, x"0f", false);
    variable many   : smalls := (others => (5, x"33", true));
    variable got    : big;
    variable u      : uc(x(1 to 5));
    variable p      : small_ptr;
    variable idx    : integer := 0;

    procedure touch is
    begin
      v.s.a := v.s.a + 100;
      behind(3)(0) := not behind(3)(0);
      sm.b(idx mod 8) := '1';
    end procedure;
    impure function look return integer is
    begin
      return v.s.a + many(1).a;
    end function;
  begin
    fill(v, 7);
    report "fill " & integer'image(sum(v)) & " " & to_hstring(v.tail);
    wait until rising_edge(clk);
    report "kept " & integer'image(sum(v));
    for i in 1 to 300 loop
      wait until rising_edge(clk);
      idx := i;
      v.r(i mod 16) := std_ulogic_vector(unsigned(v.r(i mod 16)) + i);
      many(i mod 4).a := many(i mod 4).a + i;
      bump(many((i + 1) mod 4));
      deep2(sm);
      touch;
      counter.add(sm);
    end loop;
    report "sum " & integer'image(sum(v)) & " look " & integer'image(look)
      & " total " & integer'image(counter.total);
    report "sm " & integer'image(sm.a) & " " & to_hstring(field_of(sm)) & " " & boolean'image(sm.c);
    waits(sm, clk, 40);
    report "after wait " & integer'image(sm.a) & " " & to_hstring(sm.b);
    waits(many(2), clk, 3);
    report "many " & integer'image(many(2).a) & " " & to_hstring(many(3).b);
    got := sr;
    report "gathered " & integer'image(sum(got)) & " " & to_hstring(got.tail);
    wait until rising_edge(clk);
    wait until rising_edge(clk);
    got := sr;
    report "gathered later " & integer'image(sum(got));
    uc_set(u);
    report "uc " & to_string(u.x) & " " & integer'image(u.y);
    p := new small'(sm);
    p.all.a := -1;
    report "copy apart " & integer'image(sm.a) & " " & integer'image(p.all.a);
    deallocate(p);
    many(0) := many(3);
    many(3).a := 0;
    report "array copy " & integer'image(many(0).a) & " " & integer'image(many(3).a);
    for i in before'range loop
      assert before(i) = x"aaaaaaaa" report "before touched" severity failure;
      assert behind(i)(31 downto 1) = before(i)(30 downto 0)
        report "behind touched" severity failure;
    end loop;
    report "done";
    done <= true;
    wait;
  end process;
end architecture;
