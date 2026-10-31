-- difftest: top=aggregate_impure_per_element std=2008 ref=hand
-- A composite expression with an impure call under others, a range or a choice list runs once
-- per associated element in constant and signal initial values too (LRM 9.3.3, #907).
-- ref=hand: sim1 runs it once under a range, sim2 fields of a record last to first; the expected
-- values are sim2's, with the fields of sim1.
entity aggregate_impure_per_element is end;
architecture a of aggregate_impure_per_element is
  type counter_t is protected
    impure function next_id return natural;
  end protected;
  type counter_t is protected body
    variable n : natural := 0;
    impure function next_id return natural is
    begin
      n := n + 1;
      return n;
    end function;
  end protected body;
  shared variable c : counter_t;
  type rec_t is record id : natural; w : natural; end record;
  impure function new_rec(w : natural) return rec_t is begin return (id => c.next_id, w => w); end function;
  type rec_vec is array (0 to 2) of rec_t;
  type rec_mat is array (0 to 1) of rec_vec;
  type rec_r is record a, b : rec_t; end record;
  type dv is array (2 downto 0) of rec_t;
  constant k : rec_vec := (others => new_rec(8));
  signal s : rec_vec := (others => new_rec(8));
  constant k2 : rec_vec := (1 => new_rec(1), others => new_rec(2));
  constant k3 : rec_vec := (0 | 2 => new_rec(1), 1 => new_rec(2));
  constant k4 : rec_vec := (0 to 1 => new_rec(1), 2 => new_rec(2));
  constant m : rec_mat := (others => (others => new_rec(3)));
  constant r : rec_r := (others => new_rec(4));
  constant d : dv := (others => new_rec(5));
begin
  process
    variable v : rec_vec := (others => new_rec(8));
    variable v2 : rec_vec;
    function im(x : rec_vec) return string is begin
      return natural'image(x(0).id) & " " & natural'image(x(1).id) & " " & natural'image(x(2).id); end;
  begin
    v2 := (others => new_rec(8));
    report "k " & im(k); report "s " & im(s); report "k2 " & im(k2); report "k3 " & im(k3); report "k4 " & im(k4);
    report "m " & im(m(0)) & " " & im(m(1));
    report "r " & natural'image(r.a.id) & " " & natural'image(r.b.id);
    report "d " & natural'image(d(2).id) & " " & natural'image(d(1).id) & " " & natural'image(d(0).id);
    report "v " & im(v); report "v2 " & im(v2);
    wait;
  end process;
end architecture;
