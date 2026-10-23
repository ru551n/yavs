-- An index constraint with static bounds outside the index subtype of the array type
-- (LRM 5.3.2.2, 6.3).
entity top is end;
architecture a of top is
begin
  process
    variable b : bit_vector(1 downto -1);
  begin
    report "RES " & integer'image(b'length);
    wait;
  end process;
end;
