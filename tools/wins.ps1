# list the app's windows; -Shot <title part> captures the first window whose title contains it
param([string]$Shot)
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System; using System.Text; using System.Runtime.InteropServices; using System.Collections.Generic;
public class WL { public delegate bool P(IntPtr h, IntPtr l); [StructLayout(LayoutKind.Sequential)] public struct R { public int L,T,Rr,B; }
 [DllImport("user32.dll")] public static extern bool EnumWindows(P p, IntPtr l); [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
 [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h); [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out R r); [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
 public static List<object[]> T() { var o=new List<object[]>(); EnumWindows((h,l)=>{ var sb=new StringBuilder(200); GetWindowText(h,sb,200); var t=sb.ToString(); if (IsWindowVisible(h) && t.EndsWith("Maestro")) { R r; GetWindowRect(h, out r); o.Add(new object[]{t,r.L,r.T,r.Rr-r.L,r.B-r.T,h}); } return true; }, IntPtr.Zero); return o; } }
"@
foreach ($w in [WL]::T()) { "window: $($w[0]) at $($w[1]),$($w[2]) $($w[3])x$($w[4])" }
if ($Shot) { $w = [WL]::T() | Where-Object { $_[0] -like "*$Shot*" } | Select-Object -First 1; if ($w) { [void][WL]::SetForegroundWindow($w[5]); Start-Sleep -Milliseconds 700; $b = New-Object System.Drawing.Bitmap ([int]$w[3]),([int]$w[4]); $g=[System.Drawing.Graphics]::FromImage($b); $g.CopyFromScreen([int]$w[1],[int]$w[2],0,0,(New-Object System.Drawing.Size ([int]$w[3]),([int]$w[4]))); $b.Save((Join-Path $PSScriptRoot '..\shot-pop.png')); "captured" } }
