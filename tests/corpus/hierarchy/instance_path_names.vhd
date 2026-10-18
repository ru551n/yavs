-- difftest: top=instance_path_names std=2008
entity leaf is end;
architecture a of leaf is
  signal s : bit;
begin
  process begin
    report "inst=" & leaf'instance_name & " path=" & leaf'path_name;
    report "sig=" & s'instance_name & " / " & s'path_name;
    wait;
  end process;
end;
entity instance_path_names is end;
architecture t of instance_path_names is
begin
  a1 : entity work.leaf;
  g : for i in 0 to 1 generate b : entity work.leaf; end generate;
end;
