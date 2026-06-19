/**
 * Onboarding is the zero-CLI first-run screen: attach the encrypted key file,
 * type the passphrase, activate. It must reject non-Phai files, keep the button
 * disabled until both inputs are valid, and surface the bridge's error message
 * (e.g. wrong passphrase) without leaving the screen.
 */
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import {
	render,
	screen,
	fireEvent,
	cleanup,
	waitFor,
} from "@testing-library/react";
import { Onboarding } from "../views/Onboarding";
import { api } from "../bridge/api";

vi.mock("../bridge/api", () => ({
	api: { activate: vi.fn() },
}));

const activate = vi.mocked(api.activate);

afterEach(cleanup);
beforeEach(() => activate.mockReset());

// jsdom does not implement Blob.text(), so stub it to mirror the browser.
const keyFile = (contents: string, name = "esposa.phai"): File => {
	const file = new File([contents], name, { type: "text/plain" });
	Object.defineProperty(file, "text", { value: async () => contents });
	return file;
};

const attach = (file: File) => {
	const input = document.querySelector(
		'input[type="file"]',
	) as HTMLInputElement;
	fireEvent.change(input, { target: { files: [file] } });
};

describe("Onboarding", () => {
	it("rejects a file that is not a Phai key", async () => {
		render(<Onboarding onActivated={vi.fn()} />);
		attach(keyFile("just some text"));
		await screen.findByText(/não parece uma chave/i);
		const button = screen.getByText("Ativar").closest("button") as HTMLButtonElement;
		expect(button.disabled).toBe(true);
	});

	it("activates with a valid key + passphrase and calls onActivated", async () => {
		activate.mockResolvedValue({ ok: true, label: "Mac Esposa" });
		const onActivated = vi.fn();
		render(<Onboarding onActivated={onActivated} />);

		attach(keyFile("PHAI1E-abc123"));
		fireEvent.change(screen.getByLabelText(/senha da chave/i), {
			target: { value: "s3nha" },
		});
		const button = screen.getByText("Ativar").closest("button") as HTMLButtonElement;
		await waitFor(() => expect(button.disabled).toBe(false));
		fireEvent.click(button);

		await waitFor(() => expect(onActivated).toHaveBeenCalledTimes(1));
		expect(activate).toHaveBeenCalledWith("PHAI1E-abc123", "s3nha");
	});

	it("keeps the activate button disabled until a key and passphrase are present", async () => {
		render(<Onboarding onActivated={vi.fn()} />);
		const button = () =>
			screen.getByText("Ativar").closest("button") as HTMLButtonElement;

		expect(button().disabled).toBe(true); // nothing entered yet

		attach(keyFile("PHAI1E-abc123"));
		await waitFor(() => expect(screen.getByText(/esposa\.phai/)).toBeTruthy());
		expect(button().disabled).toBe(true); // key but no passphrase

		fireEvent.change(screen.getByLabelText(/senha da chave/i), {
			target: { value: "s3nha" },
		});
		await waitFor(() => expect(button().disabled).toBe(false));
	});
});
