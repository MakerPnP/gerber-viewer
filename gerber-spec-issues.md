# Gerber spec issues

This document lists some issues encountered with various in-the-wild files found while
implementing the gerber viewer, or with the specification itself.

Spec issues have the format "SPEC-ISSUE: <camel-case-issue-tag>".  This string can be searched for in code and other files. 

In the code, issues are marked with a comment in the format:

```
// SPEC-ISSUE: <camel-case-issue-tag> [- <comment>]
```

In gerber files, issues are marked with a G04 comment in the format:

```
G04 SPEC-ISSUE: <camel-case-issue-tag> [- <comment>]*
```


## SPEC-ISSUE: closed-vs-unclosed-regions - EasyEDA v6.5.48 does not close regions properly

Gerber spec 2024.05 - "4.10 Region Statement (G36/G37)"

"A contour can only be finished if it is closed, meaning that the last vertex exactly coincides with the first one"

Indeed, the gerber spec has an example: "4.10.4.1 A Simple Contour", which specifically details
the coordinates of the first and last vertex.

EasyEDA v6.5.48 does not repeat the coordinate used in the first D02 command.

Example:
```
%LPD*%
G36*
X262960Y-2792280D02*
G01*
X260980Y-2794260D01*
X260980Y-2825740D01*
...
X296420Y-2794260D01*
X294440Y-2792280D01* <-- NOT THE SAME COORDINATE AS THE D02 ABOVE
G37*
```

The example "4.10.4.1 A Simple Contour" specifically highlights the first and last coordinates as (2,3)

```
G36* Begins a region statement
X200000Y300000D02*                 Set the current point to (2, 3), beginning a contour.
G01*                               Set linear plot mode
X700000D01*                        Create linear segment to (7, 3)
...
X200000D01*                        Create linear segment to (2, 7)
Y300000D01*                        Create linear segment to (2, 3), closing the contour. <-- THIS!
G37* Create the region by filling the contour
```

The example "4.10.4.2 Use D02 to Start a Second Contour" also closes the contours.

```
G04 Non-overlapping contours*
%MOMM*%
%FSLAX26Y26*%
%ADD10C,1.00000*%
G01*
%LPD*%
G36*
X0Y5000000D02*          <-- (0,5)
Y10000000D01*
X10000000D01*
Y0D01*
X0D01*
Y5000000D01*            <-- X is currently 0, so point is (0,5)     CONTOUR CLOSED
X-1000000D02*           <-- Y is currently 5, so point is (-10, 5)
X-5000000Y1000000D01*
X-9000000Y5000000D01*
X-5000000Y9000000D01*
X-1000000Y5000000D01*   <-- (-10, 5)                                CONTOUR CLOSED
G37*
M02*
```

Thus, it is concluded that EasyEDA v6.5.48 does not generate correct gerber files.
