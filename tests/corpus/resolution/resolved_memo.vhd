-- difftest: top=resolved_memo std=2008 stop=100ns
-- (The literals avoid '0' and '1': #433.)
-- A user resolution function is pure, so its results may be remembered; a call that reports
-- is not (the report must be made every time), and equal inputs keep giving equal results.
entity resolved_memo is end;
architecture t of resolved_memo is
  type lv is ('e', 'a', 'b', 'z');
  type lv_vec is array (natural range <>) of lv;
  function res (s : lv_vec) return lv is
    variable r : lv := 'z';
  begin
    for i in s'range loop
      if s(i) /= 'z' then
        if r = 'z' then r := s(i);
        elsif r /= s(i) then r := 'e'; report "conflict" severity note;
        end if;
      end if;
    end loop;
    return r;
  end;
  subtype rlv is res lv;
  signal b0, b1 : rlv;
  signal a0, a1 : lv := 'z';
begin
  b0 <= a0;
  b1 <= a1;
  b0 <= 'b';
  b1 <= 'z';
  process
  begin
    for i in 1 to 4 loop
      wait for 10 ns; a0 <= 'b'; a1 <= 'b';
      wait for 10 ns; a0 <= 'a'; a1 <= 'b';
      wait for 10 ns; a0 <= 'z'; a1 <= 'z';
    end loop;
    wait;
  end process;
  process (b0, b1) begin
    report lv'image(b1) & lv'image(b0) & " at " & time'image(now);
  end process;
end;
