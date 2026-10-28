-- LRM 5.3.2.2, 14.4.2.1: elaborating the declaration of c elaborates its subtype indication,
-- whose index range 0 to 2 lies outside the index subtype POSITIVE of STRING: an error,
-- although nothing reads c (#825).
entity top is end entity;
architecture a of top is
  function n return integer is begin return 0; end function;
  constant c : string(n to 2) := "abc";
begin
  process
  begin
    report "RES elaborated";
    wait;
  end process;
end architecture;
