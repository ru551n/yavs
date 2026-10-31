-- difftest: top=aggregate_others_impure std=2008 ref=hand
-- An others expression with an impure call runs once per element the other choices leave
-- (LRM 9.3.3, #909). ref=hand: sim1 runs a range choice's expression once and counts others on
-- every element; the expected values are sim2's.
entity aggregate_others_impure is end;
architecture a of aggregate_others_impure is
  type counter_t is protected
    impure function next_id return natural;
  end protected;
  type counter_t is protected body
    variable n : natural := 0;
    impure function next_id return natural is begin n := n + 1; return n; end function;
  end protected body;
  shared variable c : counter_t;
  type rec_t is record id : natural; end record;
  impure function new_rec return rec_t is begin return (id => c.next_id); end function;
  type rec_vec is array (0 to 4) of rec_t;
begin
  process
    function im(x : rec_vec) return string is begin
      return natural'image(x(0).id) & " " & natural'image(x(1).id) & " " & natural'image(x(2).id) & " " & natural'image(x(3).id) & " " & natural'image(x(4).id); end;
    variable v : rec_vec;
  begin
    v := (0 to 1 => new_rec, others => new_rec); report im(v);
    v := (others => new_rec); report im(v);
    v := (2 => (id => 77), others => new_rec); report im(v);
    report integer'image(c.next_id);
    wait;
  end process;
end;
