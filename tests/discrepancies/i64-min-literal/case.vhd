-- LRM 9.3, 5.2.3: the minus applies to the abstract literal 9223372036854775808.
entity top is end entity;
architecture a of top is
  constant k : time := -9223372036854775808 fs;
begin
  process begin report "RES ok"; wait; end process;
end architecture;
