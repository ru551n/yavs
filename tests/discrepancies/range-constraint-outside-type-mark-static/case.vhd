-- A range constraint with static bounds outside the range of its type mark (LRM 5.2.1, 6.3).
entity top is end;
architecture a of top is
begin
  process
    variable x : positive range 0 to 3;
  begin
    report "x = " & integer'image(x);
    wait;
  end process;
end;
