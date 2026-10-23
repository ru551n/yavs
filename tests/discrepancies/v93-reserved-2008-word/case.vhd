entity top is end;
architecture a of top is
  signal default, context, force : integer := 1;
begin
  process begin report "RES " & integer'image(default + context + force); wait; end process;
end;
