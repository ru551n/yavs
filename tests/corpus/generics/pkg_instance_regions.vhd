-- difftest: top=pkg_instance_regions std=2008
-- Generic package instances in every kind of declarative region.
package counter_g is
  generic (start : integer; step : integer := 1);
  function next_of(x : integer) return integer;
  constant first : integer := start + step;
end package;
package body counter_g is
  function next_of(x : integer) return integer is
  begin
    return x + step;
  end function;
end package body;

package outer is
  package inner is new work.counter_g generic map (start => 100, step => 7);
  constant from_inner : integer := inner.first;
end package;

package with_body is
  function via_body(x : integer) return integer;
end package;
package body with_body is
  package in_body is new work.counter_g generic map (start => 0, step => 11);
  function via_body(x : integer) return integer is
  begin
    return in_body.next_of(x) + in_body.first;
  end function;
end package body;

entity leaf is
  generic (w : integer);
end entity;
architecture a of leaf is
  package by_generic is new work.counter_g generic map (start => w, step => w);
begin
  process
  begin
    report "leaf w=" & integer'image(w) & " first=" & integer'image(by_generic.first)
      & " next=" & integer'image(by_generic.next_of(w));
    wait;
  end process;
end architecture;

entity pkg_instance_regions is
  package in_entity is new work.counter_g generic map (start => 1, step => 2);
end entity;
architecture t of pkg_instance_regions is
  package in_arch is new work.counter_g generic map (start => 5);
  -- Instance constants and functions used during elaboration.
  constant at_elab : integer := in_arch.next_of(in_entity.first) + work.outer.inner.first;
  signal width_vec : bit_vector(in_arch.first - 1 downto 0);

begin
  u1 : entity work.leaf generic map (w => in_entity.next_of(1));
  u2 : entity work.leaf generic map (w => 9);

  b : block
    package in_block is new work.counter_g generic map (start => 50, step => 5);
  begin
    process
    begin
      wait for 1 ns;
      report "block first=" & integer'image(in_block.first);
      wait;
    end process;
  end block;

  g : for i in 1 to 2 generate
    gb : block
      package in_gen is new work.counter_g generic map (start => i * 1000, step => i);
    begin
      process
      begin
        wait for 2 ns * i;
        report "gen " & integer'image(i) & " first=" & integer'image(in_gen.first)
          & " next=" & integer'image(in_gen.next_of(0));
        wait;
      end process;
    end block;
  end generate;

  eg : for j in 1 to in_entity.first - 1 generate
    process
    begin
      wait for (5 + j) * 100 ps;
      report "elab gen " & integer'image(j);
      wait;
    end process;
  end generate;

  process
  begin
    wait for 10 ns;
    report "at_elab=" & integer'image(at_elab) & " width=" & integer'image(width_vec'length);
    report "entity first=" & integer'image(in_entity.first);
    report "arch first=" & integer'image(in_arch.first) & " next=" & integer'image(in_arch.next_of(1));
    report "outer inner=" & integer'image(work.outer.from_inner)
      & " " & integer'image(work.outer.inner.next_of(1));
    report "body=" & integer'image(work.with_body.via_body(1));
    wait;
  end process;
end architecture;
