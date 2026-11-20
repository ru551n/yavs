-- difftest: top=copy_back_range_individual std=2008 exit=1 ref=hand
-- ref=hand for the message: sim1 and sim2 give this result, sim1 names the variable instead of
-- the formal. A record formal associated field by field: each field's value goes
-- back into its actual, checked against the actual's subtype (LRM 4.2.2.2, 6.5.7.1, #1175).
entity copy_back_range_individual is end entity;
architecture a of copy_back_range_individual is
  type rec is record
    f : integer;
    g : integer;
  end record;
  procedure set (variable o : out rec; v : integer) is begin o := (v, v + 1); end procedure;
begin
  process
    variable q : integer range 0 to 9 := 0;
    variable g : integer := 0;
  begin
    set(o.f => q, o.g => g, v => 9);
    report "q=" & integer'image(q) & " g=" & integer'image(g);
    set(o.f => q, o.g => g, v => 10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
