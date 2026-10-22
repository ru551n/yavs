-- difftest: top=tb_region_shared_elab std=2008
-- A function that reads a protected shared variable declared in an architecture is called while
-- the design is elaborated (constants, generic map actuals of instances): each instance has its
-- own variable, the calls see each other's effects in order, and the processes go on with the
-- state elaboration left.
entity leaf is
  generic (base : natural; tag : string);
end entity;

architecture a of leaf is
  type counter_t is protected
    impure function next_value return natural;
  end protected;
  type counter_t is protected body
    variable n : natural := 0;
    impure function next_value return natural is
    begin
      n := n + 1;
      return n;
    end function;
  end protected body;
  shared variable cnt : counter_t;

  impure function pick return natural is
  begin
    return base + cnt.next_value;
  end function;
  constant first : natural := pick;
  constant second : natural := pick;
begin
  process
  begin
    wait for 1 ns + base * 1 ps;
    report tag & " first=" & natural'image(first) & " second=" & natural'image(second)
      & " next=" & natural'image(cnt.next_value);
    wait;
  end process;
end architecture;

entity tb_region_shared_elab is end entity;

architecture t of tb_region_shared_elab is
  type counter_t is protected
    impure function next_value return natural;
  end protected;
  type counter_t is protected body
    variable n : natural := 0;
    impure function next_value return natural is
    begin
      n := n + 1;
      return n;
    end function;
  end protected body;
  shared variable rnd : counter_t;

  impure function seed return natural is
  begin
    return 100 * rnd.next_value;
  end function;
  constant c1 : natural := seed;
  constant c2 : natural := seed;
begin
  a : entity work.leaf generic map (base => c1, tag => "a");
  b : entity work.leaf generic map (base => c2, tag => "b");

  process
  begin
    wait for 2 ns;
    report "tb next=" & natural'image(rnd.next_value);
    wait;
  end process;
end architecture;
