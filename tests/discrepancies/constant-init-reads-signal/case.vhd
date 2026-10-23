-- A constant whose initial value reads a signal (LRM 14.4.2.2, 14.5.2: unclear).
entity top is end;
architecture a of top is
  signal s : bit_vector(3 downto 0) := "1010";
  signal n : integer := 7;
  constant c : bit_vector(3 downto 0) := s;
  constant d : integer := n + 1;
begin
  process
  begin
    report "RES " & to_string(c) & " " & integer'image(d);
    wait;
  end process;
end;
