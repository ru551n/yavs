-- A clocked process on a record variable and a procedure taking it: the variable lives in the
-- process's frame, so no statement looks up a heap object (#1048).
entity frame_vars is end;
architecture t of frame_vars is
  type rec is record
    a, b, c, d : bit;
    n : integer;
    v : bit_vector(15 downto 0);
  end record;
  signal clk : bit := '0';
  procedure step (x : inout rec) is
  begin
    x.a := x.b xor x.c;
    x.b := x.c xor x.d;
    x.n := x.n + 1;
  end procedure;
begin
  clk <= not clk after 5 ns when now < 2 us;
  process (clk)
    variable r : rec := ('0', '1', '1', '0', 0, x"00ff");
  begin
    if clk = '1' then
      r.c := r.a xor r.d;
      r.d := r.a xor r.b;
      r.v(r.n mod 16) := r.a;
      step(r);
      if r.n = 200 then
        report "n=" & integer'image(r.n) & " a=" & bit'image(r.a);
      end if;
    end if;
  end process;
end;
