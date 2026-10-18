-- difftest: top=stack std=2008
entity stack is end;
architecture t of stack is
  type stack_t is protected
    procedure push(v : integer);
    impure function pop return integer;
    impure function size return natural;
  end protected;
  type stack_t is protected body
    type arr is array (0 to 15) of integer;
    variable data : arr;
    variable n : natural := 0;
    procedure push(v : integer) is begin data(n) := v; n := n + 1; end;
    impure function pop return integer is begin n := n - 1; return data(n); end;
    impure function size return natural is begin return n; end;
  end protected body;
begin
  process
    variable s : stack_t;
  begin
    for i in 1 to 5 loop s.push(i * 3); end loop;
    report "size=" & integer'image(s.size);
    while s.size > 0 loop report "pop " & integer'image(s.pop); end loop;
    wait;
  end process;
end;
