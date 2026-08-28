/**
 * Turns arbitrary text into a QR-code module matrix using `qrcode-generator`
 * (pure JS, no dependencies, no Canvas/DOM/SVG requirement — see the
 * package-choice note in `../components/QrCode.tsx`, which renders this
 * matrix as a grid of plain `View`s). Split out into its own RN-free file so
 * it's directly unit-testable, same reasoning as `qrLivenessChallenge.ts`
 * and `faceMatchGating.ts`.
 */
import qrcode from 'qrcode-generator';

/**
 * `true` = a dark ("on") module, `false` = light ("off"). `matrix[row][col]`,
 * both 0-indexed; the matrix is always square (`matrix.length === matrix[0].length`).
 */
export type QrMatrix = boolean[][];

/**
 * Builds the module matrix for `text`.
 *
 * Type number `0` lets the library auto-pick the smallest QR version that
 * fits `text` (our payloads are short — a ~35-byte prefixed hex nonce, see
 * `qrLivenessChallenge.ts` — so this comes out small regardless). Error
 * correction level `M` (~15% recoverable) balances scan robustness against
 * module count/density; this app's payloads have no need for `H` (25%, meant
 * for codes that might get partially obscured, e.g. with a printed logo).
 */
export function buildQrMatrix(text: string): QrMatrix {
  const qr = qrcode(0, 'M');
  qr.addData(text);
  qr.make();
  const size = qr.getModuleCount();
  const matrix: QrMatrix = [];
  for (let row = 0; row < size; row++) {
    const cells: boolean[] = [];
    for (let col = 0; col < size; col++) {
      cells.push(qr.isDark(row, col));
    }
    matrix.push(cells);
  }
  return matrix;
}
