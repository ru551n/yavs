entity top is end;
architecture a of top is
  constant n : integer := 2;
begin
  g : if n = 1 generate
    process begin report "RES one"; wait; end process;
  elsif n = 2 generate
    process begin report "RES two"; wait; end process;
  end generate;
end;
