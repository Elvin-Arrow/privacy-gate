# User story — Privacy Gate

## Persona. Aisha, the immigration-application gatherer

Aisha is 34 and lives in Manchester. She works as a hospital administrator and is putting
together a spouse-visa application for her husband Omar, who is currently in Lahore. The
application needs a stack of documents. Three months of her payslips, a bank statement, a
letter from her employer confirming her salary and tenure, a tenancy agreement, and a GP letter
about a recent course of treatment that affects her declared income gaps.

Aisha is not paranoid, but she is careful. She has already done one round of this application
and it was rejected over a missing checklist item. She does not want to pay an immigration
advisor GBP 800 to read these documents for her when most of the work is just checking they are
consistent and complete. She also does not want to paste a raw bank statement into a chatbot
that trains on its inputs, and she does not want to email her full GP letter to a stranger who
might forward it on.

## The problem Aisha hits

Aisha has two distinct needs that today's tools treat as the same thing.

The first is that she wants AI help understanding her own paperwork. Are the three payslips
consistent with the employer letter? Does the bank statement show a salary deposit that matches
the payslip net pay? Is anything in the GP letter going to contradict the income gaps she
declared? She wants a smart reader she can trust with the content, without handing the content
over wholesale.

The second is that she needs to share some of these documents with a person, a visa-sponsor
caseworker at a local charity who has offered to look over the bundle for free. The caseworker
needs to see the payslips and the employer letter, but absolutely does not need Aisha's bank
account number, her GP's name and address, or the treatment details in the GP letter. Aisha has
no easy way to produce a version of each document that is safe to forward. Today she would
either black things out by hand in a PDF editor and hope she caught everything, or send the
full document and feel uneasy about it for weeks.

## How Aisha uses Privacy Gate

Aisha installs Privacy Gate on her laptop. On first run she creates an account and sets an
unlock passphrase. The app generates an encryption key that never leaves her machine.

She imports her documents one at a time. The first import asks her to set a default for whether
to keep originals. Discard is suggested; she accepts it. For each later file, the on-device
model runs and surfaces a list of detected sensitive fields, each with a label and a highlighted
span in the document.

For the bank statement, the model flags her account number, sort code, home address, and a
recurring direct-debit reference that happens to contain a partial NHS number. Aisha reviews
these. She wants the caseworker to see her name and the salary-deposit line, so she approves
those. She redacts the account number, sort code, and the NHS-number-bearing reference. She
leaves her address visible because the caseworker needs to confirm residency. The app stores
the approved version, encrypted, in the vault.

At import time Aisha is also asked whether to keep the original. For the bank statement she
overrides the discard default and keeps it, because she may need to produce a differently-redacted
version later (her accountant will want the account number but not the address). For the GP
letter she leaves the default: discard the original after the approved version is made. She does
not want the treatment details sitting on her laptop in any form once she has decided what the
caseworker is allowed to see.

Aisha now has a small set of approved versions in her vault. She goes to share them.

## Sharing to a person

Aisha selects the approved bank statement, the three approved payslips, and the approved
employer letter, and exports them as a single PDF bundle for the caseworker. Before exporting,
the app shows her a preview of exactly what will leave. She notices the employer letter still
contains her national insurance number, which the model had flagged but she had approved
earlier. For this share only she overrides that decision and redacts it. The app warns her this
override is ephemeral. She is fine with that, the canonical approved version in the vault stays
as it was, this is a one-off for the caseworker. She exports the bundle and emails it from her
own mail client. The app logs the export in the audit trail.

## Sharing to an AI

Separately, Aisha wants AI help checking internal consistency. She selects the three payslips
and the bank statement and runs the Cloud AI plugin on them. The plugin sends only the approved
content of those four documents to the cloud model and asks it to check whether the salary
deposits match the payslip net figures and flag any gaps. The model comes back with a short
report. Two of the three months match exactly. One month the deposit is GBP 40 lower than the
payslip net, which the model suggests could be a salary-sacrifice pension contribution not shown
on the payslip. Aisha checks, and it is. The model also drafts a short cover-note paragraph she
can include with the bundle, explaining the gap. None of her account numbers, sort code, or
NHS number left the device, because none of those fields were in the approved versions.

## The audit trail

Before she sends anything, Aisha opens the audit trail. She can see, for each document, what
was detected, what she approved, what she redacted, what was exported (and to whom, in her own
notes), and what was sent to the AI plugin. She can see that the GP letter original was
discarded after the approved version was made. This is the thing she could not get from any
existing tool. A verifiable answer to "what did I actually share, and what is still on my
machine?"

## Why this is better than today

Aisha did not have to trust a chatbot with her raw bank statement. She did not have to
black things out by hand and hope. She did not have to send the caseworker her GP's name or her
national insurance number. She got AI help on the consistency check without the AI ever seeing
the fields she chose to redact. And when the application is done, she can see exactly what left
her machine and to whom, and she can delete the vault contents knowing the GP letter original
was never retained in the first place.

That is the shape of the product. A consent step the user controls, applied per field, per
document, per share, with a record of what happened.