-- difftest: top=generic_file_package std=2019 ref=hand
-- A file object of a generic file type in a generic package (6.5.3, #915). ref=hand: sim1 has
-- no file operations for a generic file type; the value read back is what was written.
package gfp_t is type ft is file of integer; end package;
package gfp_p is
  generic (type designated_subtype; type file_type is file of designated_subtype; name : string);
  file f : file_type;
  procedure put(v : designated_subtype);
end package;
package body gfp_p is
  procedure put(v : designated_subtype) is begin
    file_open(f, name, write_mode);
    write(f, v);
    file_close(f);
  end procedure;
end package body;
use work.gfp_t.all;
package gfp_q is new work.gfp_p generic map (designated_subtype => integer, file_type => ft, name => "generic_file_package.bin");
use work.gfp_t.all;
entity generic_file_package is end;
architecture a of generic_file_package is
begin
  process
    file g : ft;
    variable n : integer;
  begin
    work.gfp_q.put(42);
    file_open(g, "generic_file_package.bin", read_mode);
    read(g, n);
    report integer'image(n);
    wait;
  end process;
end architecture;
