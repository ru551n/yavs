entity external_mismatch is end;
architecture a of external_mismatch is
  signal s : bit_vector(3 downto 0) := x"5";
begin
  process
    alias e is <<signal .external_mismatch.s : bit_vector(7 downto 0)>>;
  begin
    wait;
  end process;
end;
