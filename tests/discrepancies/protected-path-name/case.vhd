-- 'path_name and 'instance_name of a protected object, read inside one of its methods (LRM
-- 2019 16.2.5 lists the path of a protected object; the audit found four different answers).
entity pathprot is end entity;

architecture test of pathprot is
  type pt is protected
    impure function p return string;
    impure function i return string;
  end protected;
  type pt is protected body
    impure function p return string is
    begin
      return pt'path_name;
    end function;
    impure function i return string is
    begin
      return pt'instance_name;
    end function;
  end protected body;
  shared variable sv : pt;
begin
  process
  begin
    report "RES path " & sv.p;
    report "RES instance " & sv.i;
    wait;
  end process;
end architecture;
