-- difftest: top=linked_list std=2008
entity linked_list is end;
architecture t of linked_list is
  type node;
  type node_ptr is access node;
  type node is record val : integer; next_n : node_ptr; end record;
begin
  process
    variable head, p, tmp : node_ptr;
    variable n : integer := 0;
  begin
    for i in 1 to 5 loop head := new node'(val => i * i, next_n => head); end loop;
    p := head;
    while p /= null loop report "node " & integer'image(p.val); n := n + p.val; p := p.next_n; end loop;
    report "sum=" & integer'image(n);
    while head /= null loop tmp := head; head := head.next_n; deallocate(tmp); end loop;
    assert head = null severity failure;
    wait;
  end process;
end;
