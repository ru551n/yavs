-- 2019 generic map in a call of an uninstantiated subprogram (LCS-2016-049).
entity top is end;
architecture a of top is
  function ident generic (type t) parameter (x : t) return t is begin return x; end;
begin
  process begin
    report "RES " & integer'image(ident generic map (t => integer) (4));
    wait;
  end process;
end;
