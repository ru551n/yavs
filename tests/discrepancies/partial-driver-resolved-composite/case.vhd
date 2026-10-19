-- LRM 4.6, 14.7.2: a record subtype resolved as a whole; the process drives only field a.
entity top is end entity;
architecture a of top is
  type rec is record a, b : integer; end record;
  type rec_vec is array (natural range <>) of rec;
  function res (v : rec_vec) return rec is
    variable r : rec := (0, 0);
  begin
    for i in v'range loop r.a := r.a + v(i).a + 100; r.b := r.b + v(i).b + 1000; end loop;
    return r;
  end function;
  subtype rrec is res rec;
  signal s : rrec := (1, 2);
begin
  process begin
    s.a <= 5;
    wait for 1 ns;
    report "RES a=" & integer'image(s.a) & " b=" & integer'image(s.b);
    wait;
  end process;
end architecture;
