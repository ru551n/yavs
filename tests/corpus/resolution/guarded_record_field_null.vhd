-- difftest: top=guarded_record_field_null std=2008 ref=hand
-- ref=hand: a process drives one field of a guarded record resolved as a whole by a static
-- name, with a null waveform element (#1160). By the LRM its driver is that field's only
-- (14.7.2); yavs gives it a source over the whole record, and the other field takes no part in
-- judging whether the source is disconnected (decisions.md "Guarded record signals"). At 1 ns
-- the field is off, so the source takes no part: the bus resolves no source, the register keeps
-- its value. sim1 never shows the values p1 drives (r.a 0, q.b "00" at 0.5 ns); sim2 stops with
-- an internal error.
entity guarded_record_field_null is
end entity;

architecture sim of guarded_record_field_null is
  type rec is record
    a : integer;
    b : bit_vector(0 to 1);
  end record;
  type recs is array (natural range <>) of rec;
  function res(x : recs) return rec is
  begin
    if x'length = 0 then
      return (-1, "11");
    end if;
    return x(x'left);
  end function;
  subtype rr is res rec;
  signal r : rr bus := (0, "00");
  signal q : rr register := (0, "00");
begin
  p1 : process is
  begin
    r.a <= 5, null after 1 ns;
    q.b <= "01", null after 1 ns;
    wait;
  end process;
  check : process is
  begin
    for i in 0 to 2 loop
      wait for 500 ps;
      report "r = " & integer'image(r.a) & " " & to_string(r.b) & ", q = " &
        integer'image(q.a) & " " & to_string(q.b);
    end loop;
    wait;
  end process;
end architecture;
