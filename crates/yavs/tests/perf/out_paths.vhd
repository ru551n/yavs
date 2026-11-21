-- Scalar out formals assigned on every path before any read (if/else in an architecture's
-- procedure, case in a process's): the call stores no default into the formal (#1184), so the
-- ops of a call are those of the body and the copy back.
entity out_paths is end entity;
architecture a of out_paths is
  procedure clip (x : out integer; v : integer) is
  begin
    if v > 0 then x := v; else x := 0; end if;
  end procedure;
begin
  process
    variable q, s : integer := 0;
    procedure pick (x : out integer; v : integer) is
    begin
      case v mod 3 is
        when 0 => x := v;
        when 1 => x := 1;
        when others => x := 2;
      end case;
    end procedure;
  begin
    for i in 1 to 200 loop
      clip(q, i - 100);
      s := s + q;
      pick(q, i);
      s := s + q;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end architecture;
