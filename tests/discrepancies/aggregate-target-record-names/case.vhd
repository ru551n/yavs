-- LRM 10.6.2.1: field names in the element associations of an aggregate target name the fields of the value's record type.
entity top is end entity;
architecture a of top is
  type rec is record a, b : integer; end record;
begin
  process
    variable x, y : integer;
  begin
    (b => y, a => x) := rec'(1, 2);
    report "RES x=" & integer'image(x) & " y=" & integer'image(y);
    wait;
  end process;
end architecture;
