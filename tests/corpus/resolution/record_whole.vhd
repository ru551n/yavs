-- difftest: top=record_whole std=2008
-- A record subtype resolved as a whole (LRM 4.6): the resolution function sees whole
-- record values, and a process that drives one field drives every field (LRM 14.7.2).
entity record_whole is end;
architecture t of record_whole is
  type rec is record b : bit; n : integer; c : character; end record;
  type rec_vec is array (natural range <>) of rec;
  function res(v : rec_vec) return rec is
    variable r : rec := ('0', 100, 'a');
  begin
    for i in v'range loop
      if v(i).b = '1' then r.b := not r.b; end if;
      r.n := r.n + v(i).n * (i + 1);
      if v(i).c > r.c then r.c := v(i).c; end if;
    end loop;
    return r;
  end;
  subtype rrec is res rec;
  signal one : rrec := ('0', 0, 'a');
  signal two : rrec := ('1', 5, 'b');
  function img(r : rec) return string is
  begin
    return bit'image(r.b) & " " & integer'image(r.n) & " " & character'image(r.c);
  end;
begin
  one <= ('1', 4, 'x') after 1 ns;
  p1 : process begin
    two <= ('1', 1, 'c') after 1 ns, ('0', 2, 'd') after 3 ns;
    wait;
  end process;
  p2 : process begin
    wait for 2 ns;
    two.n <= 10;
    wait for 2 ns;
    two.b <= '0';
    wait;
  end process;
  process (one) begin report "one " & img(one); end process;
  process (two) begin report "two " & img(two); end process;
  process begin
    wait for 5 ns;
    assert one = ('1', 104, 'x') and two = ('0', 122, 'd') severity failure;
    report "done";
    wait;
  end process;
end;
