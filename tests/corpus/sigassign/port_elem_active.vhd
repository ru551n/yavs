-- difftest: top=tb_port_elem_active std=2008
-- 'active of an element of a signal that is the actual of an out port holds only for the
-- elements the process drove in that cycle, not for the whole port.
entity sub_pea is
  port (o : out bit_vector(1 to 3));
end;
architecture t of sub_pea is
  procedure write1(signal x : out bit) is
  begin
    x <= '1';
  end procedure;
begin
  p1 : process begin
    write1(o(1));
    wait for 1 ns;
    write1(o(3));
    wait;
  end process;
end;

entity tb_port_elem_active is end;
architecture t of tb_port_elem_active is
  signal x : bit_vector(1 to 3);
begin
  uut : entity work.sub_pea port map (x);
  main : process begin
    wait on x;
    assert now = 0 ns and x(1)'active and x'active;
    wait on x;
    assert now = 1 ns;
    assert x'active;
    assert not x(1)'active;
    assert not x(2)'active;
    assert x(3)'active;
    assert x = "101";
    report "ok";
    wait;
  end process;
end;
