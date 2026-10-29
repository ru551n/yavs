-- LRM 9.3.6, 5.2.3.1: the literals of 2 ** 31 - 1 convert to INTEGER, so the operations are
-- INTEGER's and 2 ** 31 lies outside it; the value of the whole static expression fits (#872).
entity top is end entity;
architecture a of top is
  signal s : natural range 0 to 2 ** 31 - 1;
begin
  process
  begin
    report "RES " & integer'image(2 ** 31 - 1);
    wait;
  end process;
end architecture;
