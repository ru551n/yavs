-- A range choice of a record aggregate whose expression calls an impure function.
entity top is end entity;
architecture a of top is
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
  type rec_vec is array (0 to 2) of rec_t;
  constant k : rec_vec := (0 to 1 => new_rec, 2 => new_rec);
begin
  process begin
    report natural'image(k(0).id) & " " & natural'image(k(1).id) & " " & natural'image(k(2).id);
    wait;
  end process;
end architecture;
