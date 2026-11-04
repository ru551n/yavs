-- difftest: top=frame_objects_first
-- Fields and descriptor words of variables in the frame read in the first activation, before any
-- suspension, where the whole objects were just written by their declarations and assignments.
entity frame_objects_first is
end entity;

architecture a of frame_objects_first is
  type small is record
    a : integer;
    b : bit_vector(7 downto 0);
    c : boolean;
  end record;
begin
  process
    variable r    : small := (4, x"0a", true);
    variable q    : small := (9, x"0b", false);
    constant line : string := "abc";
    variable n    : integer := 0;
  begin
    n := r.a + q.a;
    r := q;
    n := n * 10 + r.a;
    r.a := 100;
    q := r;
    n := n * 10 + q.a;
    for i in line'range loop
      n := n + character'pos(line(i)) * i;
    end loop;
    report integer'image(n) & " " & to_hstring(q.b) & " " & boolean'image(q.c);
    wait;
  end process;
end architecture;
