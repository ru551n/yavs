entity top is end;
architecture a of top is
  function f(x : integer; s : severity_level) return integer is
  begin
    report "RES elab report" severity s;
    return x;
  end;
  signal sig : integer := f(1, error);
begin
  process begin
    report "RES run start " & integer'image(sig);
    wait for 1 ns;
    report "RES run end";
    wait;
  end process;
end;
