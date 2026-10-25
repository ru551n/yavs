-- difftest: top=instance_name_generic ref=hand
-- ref=hand: the expected text is the LRM name forms, which the references format differently.
-- An instance label is visible in the generic map of its own instantiation (LRM 16.2.5):
-- 'instance_name and 'path_name of the label evaluate to the instance's own name (#619).
entity instance_name_sub is generic (s : string); end entity;
architecture rtl of instance_name_sub is begin
  process begin report "generic = " & s; wait; end process;
end architecture;

entity instance_name_generic is end entity;
architecture sim of instance_name_generic is
begin
  dut_0 : entity work.instance_name_sub generic map (s => dut_0'instance_name);
  dut_1 : entity work.instance_name_sub generic map (s => dut_1'path_name);
end architecture;
