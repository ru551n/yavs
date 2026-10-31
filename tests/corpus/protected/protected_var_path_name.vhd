-- difftest: top=protected_var_path_name std=2019 ref=hand
-- 'path_name and 'instance_name of a variable of a protected body go through the object that
-- holds it: in a process, an architecture, a generate and a package (LCS-2016-032, #916).
-- ref=hand: the reference runs name the library lib in this harness; the expected text says work.
package pack_pvp is
  type type_t is protected
    procedure increment;
  end protected;
end package;
package body pack_pvp is
  type type_t is protected body
    variable value : natural;
    procedure increment is
    begin
      report "path:     " & value'path_name;
      report "instance: " & value'instance_name;
      value := value + 1;
    end procedure;
  end protected body;
end package body;
package pk2_pvp is
  shared variable s : work.pack_pvp.type_t;
end package;
entity e_pvp is end;
architecture arch of e_pvp is
  shared variable sh : work.pack_pvp.type_t;
begin
  tb : process
    variable t : work.pack_pvp.type_t;
  begin
    t.increment;
    sh.increment;
    wait;
  end process;
end architecture;
entity protected_var_path_name is end entity;
architecture arch of protected_var_path_name is
begin
  u : entity work.e_pvp;
  g : for i in 0 to 1 generate
    p : process
      variable t : work.pack_pvp.type_t;
    begin
      t.increment;
      work.pk2_pvp.s.increment;
      wait;
    end process;
  end generate;
end architecture;
