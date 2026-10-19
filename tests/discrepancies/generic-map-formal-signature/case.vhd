-- 2019 signature in the formal part of a generic map (LCS-2016-I03).
entity top is end;
architecture a of top is
  function inc (x : integer) return integer is begin return x + 1; end;
  function inc (x : real) return real is begin return x + 1.0; end;
  function ap generic (function f (x : integer) return integer) parameter (x : integer)
    return integer is begin return f(x); end;
  function api is new ap generic map (f [integer return integer] => inc);
begin
  process begin report "RES " & integer'image(api(1)); wait; end process;
end;
