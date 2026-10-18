-- difftest: top=r std=2008
package p is
  type ptr_t is record ref : integer; end record;
  type pool_t is record p_data : ptr_t; end record;
  impure function new_ptr(n : natural) return ptr_t;
  impure function new_pool return pool_t;
  impure function get_pool return pool_t;
end package;
package body p is
  impure function new_ptr(n : natural) return ptr_t is
  begin
    return (ref => n);
  end;
  impure function new_pool return pool_t is
    variable pool : pool_t;
  begin
    pool.p_data := new_ptr(3);
    return pool;
  end;
  constant int_pool : pool_t := new_pool;
  impure function get_pool return pool_t is
  begin
    return int_pool;
  end;
end package body;
use work.p.all;
entity r is end;
architecture t of r is
begin
  process
    variable q : pool_t;
  begin
    q.p_data := new_ptr(7);
    report integer'image(q.p_data.ref) & " " & integer'image(get_pool.p_data.ref);
    wait;
  end process;
end;
