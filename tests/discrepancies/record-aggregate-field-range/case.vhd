-- LRM 9.3.3.2, 5.2.1: a record aggregate whose scalar element lies outside the field's range,
-- which depends on a generic, fails when evaluated, although the aggregate is locally static.
entity top is
  generic (n : natural := 5);
end entity;
architecture a of top is
  type rec is record
    k : integer range 0 to n;
  end record;
begin
  process
    variable v : rec;
  begin
    v := (k => 5);
    report "RES good " & integer'image(v.k);
    v := (k => 6);
    report "RES not reached";
    wait;
  end process;
end architecture;
