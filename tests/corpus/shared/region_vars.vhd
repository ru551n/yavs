-- difftest: top=region_vars relaxed
-- VHDL-93 style shared variables of non-protected types declared in an architecture, a block
-- and a generate (accepted with the relaxed switch): one object per elaboration of the
-- declarative part, so each instance and each for generate iteration has its own, and the
-- processes of nested blocks use the enclosing region's object. Reached directly, through a
-- subprogram of the architecture and through an alias. Same-cycle writers only add, and every
-- report has a time of its own.
entity region_vars_cell is
  generic (id : integer);
end entity;

architecture a of region_vars_cell is
  type rec_t is record
    hits : integer;
    tag  : string(1 to 3);
  end record;
  shared variable hits : integer := id * 100;
  shared variable rec : rec_t := (0, "---");
  shared variable hist : integer_vector(1 to 4) := (others => 0);
  procedure hit(n : integer) is
  begin
    hits := hits + n;
    rec.hits := rec.hits + 1;
    hist(n) := hist(n) + 1;
  end procedure;
begin
  p1 : process
  begin
    hit(1);
    wait;
  end process;

  b : block
    shared variable local : natural := 0;
  begin
    p2 : process
      alias h is hits;
    begin
      hit(2);
      h := h + 1000;
      local := local + 1;
      wait;
    end process;

    nested : block
    begin
      p3 : process
      begin
        local := local + 1;
        rec.tag := "abc";
        wait for id * 10 ns;
        report "cell " & integer'image(id) & ": local=" & integer'image(local);
        assert local = 2 severity failure;
        wait;
      end process;
    end block;
  end block;

  g : for i in 1 to 2 generate
    shared variable gv : integer := i;
  begin
    gp : process
    begin
      gv := gv * 10;
      hit(3);
      wait for id * 10 ns + i * 1 ns + 2 ns;
      report "cell " & integer'image(id) & " gen " & integer'image(i) & ": gv=" & integer'image(gv);
      assert gv = i * 10 severity failure;
      wait;
    end process;
  end generate;

  check : process
  begin
    wait for id * 10 ns + 5 ns;
    report "cell " & integer'image(id) & ": hits=" & integer'image(hits) &
      " rec.hits=" & integer'image(rec.hits) & " tag=" & rec.tag;
    assert hits = id * 100 + 1 + 2 + 1000 + 3 + 3 severity failure;
    assert rec.hits = 4 severity failure;
    assert hist = (1, 1, 2, 0) severity failure;
    assert rec.tag = "abc" severity failure;
    wait;
  end process;
end architecture;

entity region_vars is
end entity;

architecture a of region_vars is
begin
  c1 : entity work.region_vars_cell generic map (id => 1);
  c2 : entity work.region_vars_cell generic map (id => 2);
end architecture;
