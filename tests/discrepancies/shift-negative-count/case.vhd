-- LRM 9.2.4: a shift with a negative count shifts the other way (sla -k is sra k).
entity top is end entity;
architecture a of top is
begin
  process
    variable v : bit_vector(3 downto 0) := "1001";
    variable n : integer := -1;
  begin
    report "RES sra " & to_string(v sra n) & " sla " & to_string(v sla n);
    wait;
  end process;
end architecture;
