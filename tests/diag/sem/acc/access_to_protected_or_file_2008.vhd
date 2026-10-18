-- yavs-test: std=2008
-- expect: E1221 9:25 an access type cannot designate protected type `pt` in VHDL-2008
-- expect: E1221 11:25 an access type cannot designate file type `ft` in VHDL-2008
-- LRM 5.4.1
package p is
  type pt is protected
    procedure set (v : integer);
  end protected;
  type pt_ptr is access pt;
  type ft is file of integer;
  type ft_ptr is access ft;
end package;
