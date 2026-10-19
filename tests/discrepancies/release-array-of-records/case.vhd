-- LRM 10.5.2.1: `release` of a whole signal of an array of records releases every scalar
-- subelement.
entity top is end entity;
architecture a of top is
  type rec is record b : bit; n : integer; end record;
  type recs is array (0 to 1) of rec;
  signal rs : recs := (others => ('0', 0));
begin
  process begin
    rs <= force (('1', 5), ('1', 6));
    wait for 1 ns;
    report "RES forced " & integer'image(rs(1).n);
    rs <= release;
    wait for 1 ns;
    report "RES released " & integer'image(rs(1).n);
    wait;
  end process;
end architecture;
